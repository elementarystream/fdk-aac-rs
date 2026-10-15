// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libSBRenc/src/nf_est.h, libSBRenc/src/nf_est.cpp.

//! Port of `nf_est.h` and `nf_est.cpp`: the noise floor estimation.

use fdk_aac_rs_fdk::common_fix::*;
use fdk_aac_rs_fdk::fdk_assert;

use crate::fram_gen::SbrFrameInfo;
use crate::invf_est::quota_at;
use crate::sbr_def::*;

mod tables;
pub use tables::*;

/// `NF_SMOOTHING_LENGTH`: Smoothing length of the noise floors.
pub const NF_SMOOTHING_LENGTH: usize = 4;

/// `NOISE_FLOOR_OFFSET_SCALING`
const NOISE_FLOOR_OFFSET_SCALING: i32 = 4;

/// `SBR_NOISE_FLOOR_ESTIMATE` (`HANDLE_SBR_NOISE_FLOOR_ESTIMATE` is
/// `&mut SbrNoiseFloorEstimate`)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SbrNoiseFloorEstimate {
    /// `prevNoiseLevels`: The previous noise levels.
    pub prev_noise_levels: [[FixpDbl; MAX_NUM_NOISE_VALUES]; NF_SMOOTHING_LENGTH],
    /// `noiseFloorOffset`: Noise floor offset, scaled with NOISE_FLOOR_OFFSET_SCALING
    pub noise_floor_offset: [FixpDbl; MAX_NUM_NOISE_VALUES],
    /// `smoothFilter`: Smoothing filter to use (`None` for `NULL`).
    pub smooth_filter: Option<&'static [FixpDbl; 4]>,
    /// `ana_max_level`: Max level allowed.
    pub ana_max_level: FixpDbl,
    /// `weightFac`: Weightening factor for the difference between orig and sbr.
    pub weight_fac: FixpDbl,
    /// `freqBandTableQmf`: Frequncy band table for the noise floor bands.
    pub freq_band_table_qmf: [i32; MAX_NUM_NOISE_VALUES + 1],
    /// `noNoiseBands`: Number of noisebands.
    pub no_noise_bands: i32,
    /// `noiseBands`: NoiseBands switch 4 bit.
    pub noise_bands: i32,
    /// `timeSlots`: Number of timeslots in a frame.
    pub time_slots: i32,
    /// `diffThres`: Threshold value to control the inverse filtering decision
    pub diff_thres: InvfMode,
}

/// `smoothingOfNoiseLevels(NoiseLevels, nEnvelopes, noNoiseBands, prevNoiseLevels,
/// pSmoothFilter, transientFlag)`: applies smoothing to the noise levels.
fn smoothing_of_noise_levels(
    noise_levels: &mut [FixpDbl],
    n_envelopes: i32,
    no_noise_bands: i32,
    prev_noise_levels: &mut [[FixpDbl; MAX_NUM_NOISE_VALUES]; NF_SMOOTHING_LENGTH],
    p_smooth_filter: &[FixpDbl; 4],
    transient_flag: i32,
) {
    let nb = no_noise_bands.max(0) as usize;
    for env in 0..n_envelopes.max(0) as usize {
        if transient_flag != 0 {
            for prev in prev_noise_levels.iter_mut() {
                prev[..nb].copy_from_slice(&noise_levels[env * nb..env * nb + nb]);
            }
        } else {
            for i in 1..NF_SMOOTHING_LENGTH {
                let (lo, hi) = prev_noise_levels.split_at_mut(i);
                lo[i - 1][..nb].copy_from_slice(&hi[0][..nb]);
            }
            prev_noise_levels[NF_SMOOTHING_LENGTH - 1][..nb]
                .copy_from_slice(&noise_levels[env * nb..env * nb + nb]);
        }

        for band in 0..nb {
            let mut accu: FixpDbl = 0;
            for i in 0..NF_SMOOTHING_LENGTH {
                accu =
                    accu.wrapping_add(f_mult_div2(p_smooth_filter[i], prev_noise_levels[i][band]));
            }
            fdk_assert!((band + env * nb) < MAX_NUM_NOISE_VALUES);
            noise_levels[band + env * nb] = accu << 1;
        }
    }
}

/// `qmfBasedNoiseFloorDetection(noiseLevel, quotaMatrixOrig, indexVector, startIndex,
/// stopIndex, startChannel, stopChannel, ana_max_level, noiseFloorOffset,
/// missingHarmonicFlag, weightFac, diffThres, inverseFilteringLevel)`: the noise floor level
/// estimation of one band (the noise levels are scaled by 0.25).
#[allow(clippy::too_many_arguments)]
fn qmf_based_noise_floor_detection(
    noise_level: &mut FixpDbl,
    quota_matrix_orig: &[FixpDbl],
    index_vector: &[i8],
    start_index: i32,
    stop_index: i32,
    start_channel: i32,
    stop_channel: i32,
    ana_max_level: FixpDbl,
    noise_floor_offset: FixpDbl,
    missing_harmonic_flag: i32,
    weight_fac: FixpDbl,
    diff_thres: InvfMode,
    inverse_filtering_level: InvfMode,
) {
    let mut scale = 0;
    let mut mean_orig: FixpDbl = 0;
    let mut mean_sbr: FixpDbl = 0;
    let inv_index = get_inv_int(stop_index - start_index);
    let inv_channel = get_inv_int(stop_channel - start_channel);
    let mut accu: FixpDbl;

    // Calculate the mean value, over the current time segment, for the original, the HFR and
    // the difference, over all channels in the current frequency range.

    if missing_harmonic_flag == 1 {
        for l in start_channel..stop_channel {
            // tonalityOrig
            accu = 0;
            for k in start_index..stop_index {
                accu = accu.wrapping_add(f_mult_div2(quota_at(quota_matrix_orig, k, l), inv_index));
            }
            mean_orig = fix_max(mean_orig, accu << 1);

            // tonalitySbr
            accu = 0;
            for k in start_index..stop_index {
                accu = accu.wrapping_add(f_mult_div2(
                    quota_at(quota_matrix_orig, k, index_vector[l as usize] as i32),
                    inv_index,
                ));
            }
            mean_sbr = fix_max(mean_sbr, accu << 1);
        }
    } else {
        for l in start_channel..stop_channel {
            // tonalityOrig
            accu = 0;
            for k in start_index..stop_index {
                accu = accu.wrapping_add(f_mult_div2(quota_at(quota_matrix_orig, k, l), inv_index));
            }
            mean_orig = mean_orig.wrapping_add(f_mult(accu << 1, inv_channel));

            // tonalitySbr
            accu = 0;
            for k in start_index..stop_index {
                accu = accu.wrapping_add(f_mult_div2(
                    quota_at(quota_matrix_orig, k, index_vector[l as usize] as i32),
                    inv_index,
                ));
            }
            mean_sbr = mean_sbr.wrapping_add(f_mult(accu << 1, inv_channel));
        }
    }

    // Small fix to avoid noise during silent passages.
    if mean_orig <= fl2fxconst_dbl((0.000976562f32 * RELAXATION_FLOAT) as f64)
        && mean_sbr <= fl2fxconst_dbl((0.000976562f32 * RELAXATION_FLOAT) as f64)
    {
        mean_orig = fl2fxconst_dbl((101.5936673f32 * RELAXATION_FLOAT) as f64);
        mean_sbr = fl2fxconst_dbl((101.5936673f32 * RELAXATION_FLOAT) as f64);
    }

    mean_orig = fix_max(mean_orig, RELAXATION);
    mean_sbr = fix_max(mean_sbr, RELAXATION);

    let diff = if missing_harmonic_flag == 1
        || inverse_filtering_level == INVF_MID_LEVEL
        || inverse_filtering_level == INVF_LOW_LEVEL
        || inverse_filtering_level == INVF_OFF
        || inverse_filtering_level <= diff_thres
    {
        RELAXATION
    } else {
        accu = f_div_norm(mean_sbr, mean_orig, &mut scale);

        // C: fMult(...) >> (RELAXATION_SHIFT - scale)
        fix_max(
            RELAXATION,
            f_mult(RELAXATION_FRACT, f_mult(weight_fac, accu))
                .wrapping_shr((RELAXATION_SHIFT - scale) as u32),
        )
    };

    // noise Level is now a positive value, i.e. the more harmonic the signal is the higher
    // noise level, this makes no sense so we change the sign.
    accu = f_div_norm(diff, mean_orig, &mut scale);
    scale -= 2;

    if scale > 0 && accu > (MAXVAL_DBL >> scale) {
        *noise_level = MAXVAL_DBL;
    } else {
        *noise_level = scale_value(accu, scale);
    }

    // Add a noise floor offset to compensate for bias in the detector
    if missing_harmonic_flag == 0 {
        *noise_level = fix_min(
            f_mult(*noise_level, noise_floor_offset),
            MAXVAL_DBL >> NOISE_FLOOR_OFFSET_SCALING,
        ) << NOISE_FLOOR_OFFSET_SCALING;
    }

    // check to see that we don't exceed the maximum allowed level
    *noise_level = fix_min(*noise_level, ana_max_level); // ana_max_level is scaled with factor 0.25
}

/// `FDKsbrEnc_sbrNoiseFloorEstimateQmf(h_sbrNoiseFloorEstimate, frame_info, noiseLevels,
/// quotaMatrixOrig, indexVector, missingHarmonicsFlag, startIndex, numberOfEstimatesPerFrame,
/// transientFrame, pInvFiltLevels, sbrSyntaxFlags)`: the noise floor estimation for the time
/// segments decided upon the transient information (one or two segments). `quota_matrix_orig`
/// is the flat quota matrix (see [`quota_at`]).
#[allow(clippy::too_many_arguments)]
pub fn fdk_sbr_enc_sbr_noise_floor_estimate_qmf(
    h: &mut SbrNoiseFloorEstimate,
    frame_info: &SbrFrameInfo,
    noise_levels: &mut [FixpDbl],
    quota_matrix_orig: &[FixpDbl],
    index_vector: &[i8],
    missing_harmonics_flag: i32,
    start_index: i32,
    number_of_estimates_per_frame: u32,
    transient_frame: i32,
    p_inv_filt_levels: &[InvfMode],
    _sbr_syntax_flags: u32,
) {
    let mut start_pos = [0i32; 2];
    let mut stop_pos = [0i32; 2];
    let no_noise_bands = h.no_noise_bands;

    let n_noise_envelopes = frame_info.n_noise_envelopes;

    start_pos[0] = start_index;

    let est = number_of_estimates_per_frame.min(2) as i32;
    if n_noise_envelopes == 1 {
        stop_pos[0] = start_index + est;
    } else {
        stop_pos[0] = start_index + 1;
        start_pos[1] = start_index + 1;
        stop_pos[1] = start_index + est;
    }

    // Estimate the noise floor.
    for env in 0..n_noise_envelopes.max(0) as usize {
        for band in 0..no_noise_bands.max(0) as usize {
            fdk_assert!((band + env * no_noise_bands as usize) < MAX_NUM_NOISE_VALUES);
            let mut level = noise_levels[band + env * no_noise_bands as usize];
            qmf_based_noise_floor_detection(
                &mut level,
                quota_matrix_orig,
                index_vector,
                start_pos[env],
                stop_pos[env],
                h.freq_band_table_qmf[band],
                h.freq_band_table_qmf[band + 1],
                h.ana_max_level,
                h.noise_floor_offset[band],
                missing_harmonics_flag,
                h.weight_fac,
                h.diff_thres,
                p_inv_filt_levels[band],
            );
            noise_levels[band + env * no_noise_bands as usize] = level;
        }
    }

    // Smoothing of the values.
    if let Some(filter) = h.smooth_filter {
        smoothing_of_noise_levels(
            noise_levels,
            n_noise_envelopes,
            h.no_noise_bands,
            &mut h.prev_noise_levels,
            filter,
            transient_frame,
        );
    }

    // quantisation
    for env in 0..n_noise_envelopes.max(0) as usize {
        for band in 0..no_noise_bands.max(0) as usize {
            let k = band + env * no_noise_bands as usize;
            fdk_assert!(k < MAX_NUM_NOISE_VALUES);
            noise_levels[k] = NOISE_FLOOR_OFFSET_64
                .wrapping_sub(calc_ld_data(noise_levels[k].wrapping_add(1)))
                .wrapping_add(QUANT_OFFSET);
        }
    }
}

/// `downSampleLoRes(v_result, num_result, freqBandTableRef, num_Ref)`: returns 0 if OK.
fn down_sample_lo_res(
    v_result: &mut [i32],
    num_result: i32,
    freq_band_table_ref: &[u8],
    num_ref: i32,
) -> i32 {
    let mut v_index = [0i32; MAX_FREQ_COEFFS / 2];

    // init
    let mut org_length = num_ref;
    let mut result_length = num_result;

    v_index[0] = 0; // Always use left border
    let mut i = 0usize;
    while org_length > 0 {
        // Create downsample vector
        i += 1;
        if result_length == 0 || i >= v_index.len() {
            // Guarded: an error where the reference has no defined result.
            return 1;
        }
        let step = org_length / result_length; // floor;
        org_length -= step;
        result_length -= 1;
        v_index[i] = v_index[i - 1] + step;
    }

    if i as i32 != num_result {
        // Should never happen
        return 1; // error downsampling
    }

    for j in 0..=i {
        // Use downsample vector to index LoResolution vector.
        v_result[j] = freq_band_table_ref[v_index[j] as usize] as i32;
    }

    0
}

/// `FDKsbrEnc_InitSbrNoiseFloorEstimate(h_sbrNoiseFloorEstimate, ana_max_level, freqBandTable,
/// nSfb, noiseBands, noiseFloorOffset, timeSlots, useSpeechConfig)`: returns 0 if OK.
#[allow(clippy::too_many_arguments)]
pub fn fdk_sbr_enc_init_sbr_noise_floor_estimate(
    h: &mut SbrNoiseFloorEstimate,
    ana_max_level: i32,
    freq_band_table: &[u8],
    n_sfb: i32,
    noise_bands: i32,
    noise_floor_offset: i32,
    time_slots: i32,
    use_speech_config: u32,
) -> i32 {
    let tmp: FixpDbl;

    *h = SbrNoiseFloorEstimate::default();

    h.smooth_filter = Some(&SMOOTH_FILTER);
    if use_speech_config != 0 {
        h.weight_fac = MAXVAL_DBL;
        h.diff_thres = INVF_LOW_LEVEL;
    } else {
        h.weight_fac = fl2fxconst_dbl(0.25f32 as f64);
        h.diff_thres = INVF_MID_LEVEL;
    }

    h.time_slots = time_slots;
    h.noise_bands = noise_bands;

    // h_sbrNoiseFloorEstimate->ana_max_level is scaled by 0.25
    h.ana_max_level = match ana_max_level {
        6 => MAXVAL_DBL,
        3 => fl2fxconst_dbl(0.5),
        -3 => fl2fxconst_dbl(0.125),
        _ => MAXVAL_DBL, // Should not enter here
    };

    // calculate number of noise bands and allocate
    if fdk_sbr_enc_reset_sbr_noise_floor_estimate(h, freq_band_table, n_sfb) != 0 {
        return 1;
    }

    if noise_floor_offset == 0 {
        tmp = MAXVAL_DBL >> NOISE_FLOOR_OFFSET_SCALING;
    } else {
        // noiseFloorOffset has to be smaller than 12, because the result of the calculation
        // below must be smaller than 1: (2^(noiseFloorOffset/3))*2^4<1
        fdk_assert!(noise_floor_offset < 12);

        // Assumes the noise floor offset in tuning table are in q31
        // Change the qformat here when non-zero values would be filled
        let mut qexp = 0;
        let mut qtmp = 0;
        let exp = f_div_norm(noise_floor_offset, 3, &mut qexp);
        let t = f_pow(2, DFRACT_BITS - 1, exp, qexp, &mut qtmp);
        tmp = scale_value(t, qtmp - NOISE_FLOOR_OFFSET_SCALING);
    }

    for i in 0..(h.no_noise_bands.max(0) as usize).min(MAX_NUM_NOISE_VALUES) {
        h.noise_floor_offset[i] = tmp;
    }

    0
}

/// `FDKsbrEnc_resetSbrNoiseFloorEstimate(h_sbrNoiseFloorEstimate, freqBandTable, nSfb)`:
/// returns 0 if OK.
pub fn fdk_sbr_enc_reset_sbr_noise_floor_estimate(
    h: &mut SbrNoiseFloorEstimate,
    freq_band_table: &[u8],
    n_sfb: i32,
) -> i32 {
    // Calculate number of noise bands
    let k2 = freq_band_table[n_sfb as usize] as i32;
    let kx = freq_band_table[0] as i32;

    if h.noise_bands == 0 {
        h.no_noise_bands = 1;
    } else {
        // Calculate number of noise bands 1,2 or 3 bands/octave
        let mut ratio_e = 0;
        let mut qlg2 = 0;
        let ratio = f_div_norm(k2, kx, &mut ratio_e);
        let lg2 = f_log2(ratio, ratio_e, &mut qlg2);
        let mut tmp = f_mult(h.noise_bands.wrapping_shl(24), lg2);
        tmp = scale_value(tmp, qlg2 - 23);

        let mut n_noise_bands = tmp.wrapping_add(1) >> 1;

        if n_noise_bands > MAX_NUM_NOISE_COEFFS as i32 {
            n_noise_bands = MAX_NUM_NOISE_COEFFS as i32;
        }

        if n_noise_bands == 0 {
            n_noise_bands = 1;
        }

        h.no_noise_bands = n_noise_bands;
    }

    down_sample_lo_res(&mut h.freq_band_table_qmf, h.no_noise_bands, freq_band_table, n_sfb)
}

/// `FDKsbrEnc_deleteSbrNoiseFloorEstimate(h_sbrNoiseFloorEstimate)`: nothing to do.
pub fn fdk_sbr_enc_delete_sbr_noise_floor_estimate(_h: &mut SbrNoiseFloorEstimate) {}
