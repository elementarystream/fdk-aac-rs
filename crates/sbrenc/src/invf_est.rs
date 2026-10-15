// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libSBRenc/src/invf_est.h, libSBRenc/src/invf_est.cpp.

//! Port of `invf_est.h` and `invf_est.cpp`: the inverse filtering level detection.
//!
//! The quota matrix of the tonality correction module (`FIXP_DBL **quotaMatrix` in C, rows of
//! 64 values in one allocation) is passed as its flat memory; see [`quota_at`].

use fdk_aac_rs_fdk::common_fix::*;

use crate::sbr_def::{InvfMode, MAX_NUM_NOISE_VALUES};
use crate::sbr_misc::fdk_sbr_enc_shellsort_fract;

mod tables;
pub use tables::*;

/// `INVF_SMOOTHING_LENGTH`
pub const INVF_SMOOTHING_LENGTH: usize = 2;

/// `MAX_NUM_REGIONS`
const MAX_NUM_REGIONS: usize = 10;

/// `DETECTOR_PARAMETERS`
#[derive(Debug, PartialEq, Eq)]
pub struct DetectorParameters {
    /// `quantStepsSbr`
    pub quant_steps_sbr: &'static [FixpDbl; 4],
    /// `quantStepsOrig`
    pub quant_steps_orig: &'static [FixpDbl; 4],
    /// `nrgBorders`
    pub nrg_borders: &'static [FixpDbl; 4],
    /// `numRegionsSbr`
    pub num_regions_sbr: i32,
    /// `numRegionsOrig`
    pub num_regions_orig: i32,
    /// `numRegionsNrg`
    pub num_regions_nrg: i32,
    /// `regionSpace`
    pub region_space: [[InvfMode; 5]; 5],
    /// `regionSpaceTransient`
    pub region_space_transient: [[InvfMode; 5]; 5],
    /// `EnergyCompFactor`
    pub energy_comp_factor: [i32; 5],
}

/// `DETECTOR_VALUES`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DetectorValues {
    /// `origQuotaMean`
    pub orig_quota_mean: [FixpDbl; INVF_SMOOTHING_LENGTH + 1],
    /// `sbrQuotaMean`
    pub sbr_quota_mean: [FixpDbl; INVF_SMOOTHING_LENGTH + 1],
    /// `origQuotaMeanStrongest`
    pub orig_quota_mean_strongest: [FixpDbl; INVF_SMOOTHING_LENGTH + 1],
    /// `sbrQuotaMeanStrongest`
    pub sbr_quota_mean_strongest: [FixpDbl; INVF_SMOOTHING_LENGTH + 1],
    /// `origQuotaMeanFilt`
    pub orig_quota_mean_filt: FixpDbl,
    /// `sbrQuotaMeanFilt`
    pub sbr_quota_mean_filt: FixpDbl,
    /// `origQuotaMeanStrongestFilt`
    pub orig_quota_mean_strongest_filt: FixpDbl,
    /// `sbrQuotaMeanStrongestFilt`
    pub sbr_quota_mean_strongest_filt: FixpDbl,
    /// `origQuotaMax`
    pub orig_quota_max: FixpDbl,
    /// `sbrQuotaMax`
    pub sbr_quota_max: FixpDbl,
    /// `avgNrg`
    pub avg_nrg: FixpDbl,
}

/// `SBR_INV_FILT_EST` (`HANDLE_SBR_INV_FILT_EST` is `&mut SbrInvFiltEst`)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SbrInvFiltEst {
    /// `numberOfStrongest`
    pub number_of_strongest: i32,
    /// `prevRegionSbr`
    pub prev_region_sbr: [i32; MAX_NUM_NOISE_VALUES],
    /// `prevRegionOrig`
    pub prev_region_orig: [i32; MAX_NUM_NOISE_VALUES],
    /// `freqBandTableInvFilt`
    pub freq_band_table_inv_filt: [i32; MAX_NUM_NOISE_VALUES],
    /// `noDetectorBands`
    pub no_detector_bands: i32,
    /// `noDetectorBandsMax`
    pub no_detector_bands_max: i32,
    /// `detectorParams` (`None` for `NULL`)
    pub detector_params: Option<&'static DetectorParameters>,
    /// `prevInvfMode`
    pub prev_invf_mode: [InvfMode; MAX_NUM_NOISE_VALUES],
    /// `detectorValues`
    pub detector_values: [DetectorValues; MAX_NUM_NOISE_VALUES],
    /// `nrgAvg`
    pub nrg_avg: FixpDbl,
    /// `wmQmf`
    pub wm_qmf: [FixpDbl; MAX_NUM_NOISE_VALUES],
}

/// `quotaMatrix[j][i]`: the element `i` of row `j` of the quota matrix (rows of 64 values in
/// one allocation; `i` may be -1, the patch index of a guard band: the last element of the
/// previous row, reproduced from the C layout). Elements outside of the allocation read as 0.
#[inline]
pub fn quota_at(quota: &[FixpDbl], j: i32, i: i32) -> FixpDbl {
    let idx = j as isize * 64 + i as isize;
    if idx < 0 {
        0
    } else {
        quota.get(idx as usize).copied().unwrap_or(0)
    }
}

/// `calculateDetectorValues(quotaMatrixOrig, indexVector, nrgVector, detectorValues,
/// startChannel, stopChannel, startIndex, stopIndex, numberOfStrongest)`
#[allow(clippy::too_many_arguments)]
fn calculate_detector_values(
    quota_matrix_orig: &[FixpDbl],
    index_vector: &[i8],
    nrg_vector: &[FixpDbl],
    detector_values: &mut DetectorValues,
    start_channel: i32,
    stop_channel: i32,
    start_index: i32,
    stop_index: i32,
    number_of_strongest: i32,
) {
    let filter = FIR_TABLE[INVF_SMOOTHING_LENGTH];
    let mut quota_vec_orig = [0 as FixpDbl; 64];
    let mut quota_vec_sbr = [0 as FixpDbl; 64];

    let inv_index = get_inv_int(stop_index - start_index);
    let inv_channel = get_inv_int(stop_channel - start_channel);

    // Calculate the mean value, over the current time segment, for the original, the HFR and
    // the difference, over all channels in the current frequency range.
    // NOTE: the averaging is done on the values quota/(1 - quota + RELAXATION).

    // The original, the sbr signal and the total energy
    detector_values.avg_nrg = 0;
    for j in start_index..stop_index {
        for i in start_channel..stop_channel {
            let iu = i as usize;
            quota_vec_orig[iu] = quota_vec_orig[iu]
                .wrapping_add(f_mult(quota_at(quota_matrix_orig, j, i), inv_index));
            if index_vector[iu] != -1 {
                quota_vec_sbr[iu] = quota_vec_sbr[iu].wrapping_add(f_mult(
                    quota_at(quota_matrix_orig, j, index_vector[iu] as i32),
                    inv_index,
                ));
            }
        }
        detector_values.avg_nrg =
            detector_values.avg_nrg.wrapping_add(f_mult(nrg_vector[j as usize], inv_index));
    }

    // Calculate the mean value, over the current frequency range, for the original, the HFR
    // and the difference. Also calculate the same mean values for the three vectors, but only
    // includeing the x strongest copmponents.

    let mut orig_quota: FixpDbl = 0;
    let mut sbr_quota: FixpDbl = 0;
    for i in start_channel as usize..stop_channel as usize {
        orig_quota = orig_quota.wrapping_add(f_mult_div2(quota_vec_orig[i], inv_channel));
        sbr_quota = sbr_quota.wrapping_add(f_mult_div2(quota_vec_sbr[i], inv_channel));
    }

    // Calculate the mean value for the x strongest components
    fdk_sbr_enc_shellsort_fract(
        &mut quota_vec_orig[start_channel as usize..],
        stop_channel - start_channel,
    );
    fdk_sbr_enc_shellsort_fract(
        &mut quota_vec_sbr[start_channel as usize..],
        stop_channel - start_channel,
    );

    let mut orig_quota_mean_strongest: FixpDbl = 0;
    let mut sbr_quota_mean_strongest: FixpDbl = 0;

    let temp = (stop_channel - start_channel).min(number_of_strongest);
    let inv_temp = get_inv_int(temp);

    for i in 0..temp.max(0) {
        let k = (i + stop_channel - temp) as usize;
        orig_quota_mean_strongest =
            orig_quota_mean_strongest.wrapping_add(f_mult_div2(quota_vec_orig[k], inv_temp));
        sbr_quota_mean_strongest =
            sbr_quota_mean_strongest.wrapping_add(f_mult_div2(quota_vec_sbr[k], inv_temp));
    }

    // The value for the strongest component
    detector_values.orig_quota_max = quota_vec_orig[(stop_channel - 1) as usize];
    detector_values.sbr_quota_max = quota_vec_sbr[(stop_channel - 1) as usize];

    // Buffer values
    detector_values.orig_quota_mean.copy_within(1.., 0);
    detector_values.sbr_quota_mean.copy_within(1.., 0);
    detector_values.orig_quota_mean_strongest.copy_within(1.., 0);
    detector_values.sbr_quota_mean_strongest.copy_within(1.., 0);

    detector_values.orig_quota_mean[INVF_SMOOTHING_LENGTH] = orig_quota << 1;
    detector_values.sbr_quota_mean[INVF_SMOOTHING_LENGTH] = sbr_quota << 1;
    detector_values.orig_quota_mean_strongest[INVF_SMOOTHING_LENGTH] =
        orig_quota_mean_strongest << 1;
    detector_values.sbr_quota_mean_strongest[INVF_SMOOTHING_LENGTH] = sbr_quota_mean_strongest << 1;

    // Filter values
    detector_values.orig_quota_mean_filt = 0;
    detector_values.sbr_quota_mean_filt = 0;
    detector_values.orig_quota_mean_strongest_filt = 0;
    detector_values.sbr_quota_mean_strongest_filt = 0;

    for i in 0..INVF_SMOOTHING_LENGTH + 1 {
        detector_values.orig_quota_mean_filt = detector_values
            .orig_quota_mean_filt
            .wrapping_add(f_mult(detector_values.orig_quota_mean[i], filter[i]));
        detector_values.sbr_quota_mean_filt = detector_values
            .sbr_quota_mean_filt
            .wrapping_add(f_mult(detector_values.sbr_quota_mean[i], filter[i]));
        detector_values.orig_quota_mean_strongest_filt = detector_values
            .orig_quota_mean_strongest_filt
            .wrapping_add(f_mult(detector_values.orig_quota_mean_strongest[i], filter[i]));
        detector_values.sbr_quota_mean_strongest_filt = detector_values
            .sbr_quota_mean_strongest_filt
            .wrapping_add(f_mult(detector_values.sbr_quota_mean_strongest[i], filter[i]));
    }
}

/// `findRegion(currVal, borders, numBorders)`: the region in which the input value belongs.
fn find_region(curr_val: FixpDbl, borders: &[FixpDbl], num_borders: i32) -> i32 {
    if curr_val < borders[0] {
        return 0;
    }

    for i in 1..num_borders as usize {
        if curr_val >= borders[i - 1] && curr_val < borders[i] {
            return i as i32;
        }
    }

    if curr_val >= borders[(num_borders - 1) as usize] {
        return num_borders;
    }

    0 // We never get here, it's just to avoid compiler warnings.
}

/// `decisionAlgorithm(detectorParams, detectorValues, transientFlag, prevRegionSbr,
/// prevRegionOrig)`: the inverse filtering mode from the quota values.
fn decision_algorithm(
    detector_params: &DetectorParameters,
    detector_values: &DetectorValues,
    transient_flag: i32,
    prev_region_sbr: &mut i32,
    prev_region_orig: &mut i32,
) -> InvfMode {
    // Current thresholds.
    let num_regions_sbr = detector_params.num_regions_sbr;
    let num_regions_orig = detector_params.num_regions_orig;
    let num_regions_nrg = detector_params.num_regions_nrg;

    let mut quant_steps_sbr_tmp = [0 as FixpDbl; MAX_NUM_REGIONS];
    let mut quant_steps_orig_tmp = [0 as FixpDbl; MAX_NUM_REGIONS];

    // Current detector values.

    // 0.375 = 3.0 / 8.0; 0.31143075889 = log2(RELAXATION)/64.0; 0.625 = log(16)/64.0;
    // 0.6875 = 44/64.0
    let orig_quota_mean_filt = f_mult_div2(
        fl2fxconst_dbl((2.0f32 * 0.375f32) as f64),
        calc_ld_data(detector_values.orig_quota_mean_filt.max(1))
            .wrapping_add(fl2fxconst_dbl(0.31143075889f32 as f64)),
    ); // scaled by 1/2^9
    let sbr_quota_mean_filt = f_mult_div2(
        fl2fxconst_dbl((2.0f32 * 0.375f32) as f64),
        calc_ld_data(detector_values.sbr_quota_mean_filt.max(1))
            .wrapping_add(fl2fxconst_dbl(0.31143075889f32 as f64)),
    ); // scaled by 1/2^9
       // If energy is zero then we will get different results for different word lengths.
    let nrg = f_mult_div2(
        fl2fxconst_dbl((2.0f32 * 0.375f32) as f64),
        calc_ld_data(detector_values.avg_nrg.wrapping_add(1))
            .wrapping_add(fl2fxconst_dbl(0.0625f32 as f64))
            .wrapping_add(fl2fxconst_dbl(0.6875f32 as f64)),
    ); // scaled by 1/2^8; 2^44 -> qmf energy scale

    let nsbr = (num_regions_sbr.max(0) as usize).min(4);
    let norig = (num_regions_orig.max(0) as usize).min(4);
    quant_steps_sbr_tmp[..nsbr].copy_from_slice(&detector_params.quant_steps_sbr[..nsbr]);
    quant_steps_orig_tmp[..norig].copy_from_slice(&detector_params.quant_steps_orig[..norig]);

    if *prev_region_sbr < num_regions_sbr {
        quant_steps_sbr_tmp[*prev_region_sbr as usize] =
            detector_params.quant_steps_sbr[*prev_region_sbr as usize].wrapping_add(HYSTERESIS);
    }
    if *prev_region_sbr > 0 {
        quant_steps_sbr_tmp[(*prev_region_sbr - 1) as usize] = detector_params.quant_steps_sbr
            [(*prev_region_sbr - 1) as usize]
            .wrapping_sub(HYSTERESIS);
    }

    if *prev_region_orig < num_regions_orig {
        quant_steps_orig_tmp[*prev_region_orig as usize] =
            detector_params.quant_steps_orig[*prev_region_orig as usize].wrapping_add(HYSTERESIS);
    }
    if *prev_region_orig > 0 {
        quant_steps_orig_tmp[(*prev_region_orig - 1) as usize] = detector_params.quant_steps_orig
            [(*prev_region_orig - 1) as usize]
            .wrapping_sub(HYSTERESIS);
    }

    let region_sbr = find_region(sbr_quota_mean_filt, &quant_steps_sbr_tmp, num_regions_sbr);
    let region_orig = find_region(orig_quota_mean_filt, &quant_steps_orig_tmp, num_regions_orig);
    let region_nrg = find_region(nrg, detector_params.nrg_borders, num_regions_nrg);

    *prev_region_sbr = region_sbr;
    *prev_region_orig = region_orig;

    // Use different settings if a transient is present
    let mut inv_filt_level = if transient_flag == 1 {
        detector_params.region_space_transient[region_sbr as usize][region_orig as usize]
    } else {
        detector_params.region_space[region_sbr as usize][region_orig as usize]
    };

    // Compensate for low energy.
    inv_filt_level =
        (inv_filt_level + detector_params.energy_comp_factor[region_nrg as usize]).max(0);

    inv_filt_level
}

/// `FDKsbrEnc_qmfInverseFilteringDetector(hInvFilt, quotaMatrix, nrgVector, indexVector,
/// startIndex, stopIndex, transientFlag, infVec)`: estimates the inverse filtering level
/// required in the decoder. `quota_matrix` is the flat quota matrix (see [`quota_at`]).
#[allow(clippy::too_many_arguments)]
pub fn fdk_sbr_enc_qmf_inverse_filtering_detector(
    h_inv_filt: &mut SbrInvFiltEst,
    quota_matrix: &[FixpDbl],
    nrg_vector: &[FixpDbl],
    index_vector: &[i8],
    start_index: i32,
    stop_index: i32,
    transient_flag: i32,
    inf_vec: &mut [InvfMode],
) {
    let Some(params) = h_inv_filt.detector_params else {
        return;
    };
    // Do the inverse filtering level estimation.
    for band in 0..h_inv_filt.no_detector_bands.max(0) as usize {
        let start_channel = h_inv_filt.freq_band_table_inv_filt[band];
        let stop_channel = h_inv_filt.freq_band_table_inv_filt[band + 1];

        calculate_detector_values(
            quota_matrix,
            index_vector,
            nrg_vector,
            &mut h_inv_filt.detector_values[band],
            start_channel,
            stop_channel,
            start_index,
            stop_index,
            h_inv_filt.number_of_strongest,
        );

        inf_vec[band] = decision_algorithm(
            params,
            &h_inv_filt.detector_values[band],
            transient_flag,
            &mut h_inv_filt.prev_region_sbr[band],
            &mut h_inv_filt.prev_region_orig[band],
        );
    }
}

/// `FDKsbrEnc_initInvFiltDetector(hInvFilt, freqBandTableDetector, numDetectorBands,
/// useSpeechConfig)`: returns 0.
pub fn fdk_sbr_enc_init_inv_filt_detector(
    h_inv_filt: &mut SbrInvFiltEst,
    freq_band_table_detector: &[i32],
    num_detector_bands: i32,
    use_speech_config: u32,
) -> i32 {
    *h_inv_filt = SbrInvFiltEst::default();

    h_inv_filt.detector_params = Some(if use_speech_config != 0 {
        &DETECTOR_PARAMS_AAC_SPEECH
    } else {
        &DETECTOR_PARAMS_AAC
    });

    h_inv_filt.no_detector_bands_max = num_detector_bands;

    // Memory initialisation
    for i in 0..(h_inv_filt.no_detector_bands_max.max(0) as usize).min(MAX_NUM_NOISE_VALUES) {
        h_inv_filt.detector_values[i] = DetectorValues::default();
        h_inv_filt.prev_invf_mode[i] = crate::sbr_def::INVF_OFF;
        h_inv_filt.prev_region_orig[i] = 0;
        h_inv_filt.prev_region_sbr[i] = 0;
    }

    // Reset the inverse fltering detector.
    fdk_sbr_enc_reset_inv_filt_detector(
        h_inv_filt,
        freq_band_table_detector,
        h_inv_filt.no_detector_bands_max,
    );

    0
}

/// `FDKsbrEnc_resetInvFiltDetector(hInvFilt, freqBandTableDetector, numDetectorBands)`:
/// returns 0.
///
/// `numDetectorBands` is at most `MAX_NUM_NOISE_COEFFS` (the number of noise bands); C copies
/// `numDetectorBands + 1` table entries into a 10-entry array.
pub fn fdk_sbr_enc_reset_inv_filt_detector(
    h_inv_filt: &mut SbrInvFiltEst,
    freq_band_table_detector: &[i32],
    num_detector_bands: i32,
) -> i32 {
    h_inv_filt.number_of_strongest = 1;
    let n = ((num_detector_bands + 1).max(0) as usize).min(MAX_NUM_NOISE_VALUES);
    h_inv_filt.freq_band_table_inv_filt[..n].copy_from_slice(&freq_band_table_detector[..n]);
    h_inv_filt.no_detector_bands = num_detector_bands;

    0
}
