// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libSBRenc/src/mh_det.h, libSBRenc/src/mh_det.cpp.

//! Port of `mh_det.h` and `mh_det.cpp`: the missing harmonics detection.
//!
//! # Memory
//!
//! The detection vectors and the guide vectors are pointers into per channel allocations of
//! `MAX_NO_OF_ESTIMATES * MAX_FREQ_COEFFS` values in C (`Ram_Sbr_detectionVectors`, ...); the
//! port owns these allocations as flat arrays, row `i` at `i * MAX_FREQ_COEFFS`.
//!
//! The tonality difference and flatness rows of the last `MAX_NO_OF_ESTIMATES / 2` estimates
//! live in a scratch buffer of `FDKsbrEnc_SbrMissingHarmonicsDetectorQmf()` in C; their first
//! `nSfb` values are written before they are read, and the encoder outputs do not depend on the
//! others. The port uses zeros there.
//!
//! # `guideVectorDetected`
//!
//! `detectionWithPrediction()` copies `nSfb * sizeof(INT)` bytes (four times the size of the
//! vector) from the missing harmonics vector `addHarmonic` of `SBR_ENV_DATA` into
//! `guideVectors[0].guideVectorDetected`: the copy reads the members of `SBR_ENV_DATA` that
//! follow `addHarmonic` and writes into the guide vectors of the next estimates. The port
//! reproduces the bytes from the caller's image of those members ([`ENV_DATA_TAIL_BYTES`],
//! built by [`crate::bit_sbr::SbrEnvData::add_harmonic_tail`]). `guideVectorDetected` is
//! never read for a decision; it only changes the detector state.

use fdk_aac_rs_fdk::common_fix::*;
use fdk_aac_rs_fdk::fdk_assert;

use crate::fram_gen::{SbrFrameInfo, FRAME_MIDDLE_SLOT_2048, FRAME_MIDDLE_SLOT_512LD};
use crate::fram_gen::{NUMBER_TIME_SLOTS_1920, NUMBER_TIME_SLOTS_2048};
use crate::invf_est::quota_at;
use crate::sbr_def::*;
use crate::sbr_encoder::SBR_SYNTAX_LOW_DELAY;
use crate::sbr_misc::fdk_sbr_enc_lsi_divide_scale_fract;

mod tables;
pub use tables::*;

/// `SFM_SHIFT`: Attention: SFM_SCALE depends on SFM_SHIFT
const SFM_SHIFT: i32 = 2;
/// `SFM_SCALE`: 1.0 >> SFM_SHIFT
const SFM_SCALE: FixpDbl = MAXVAL_DBL >> SFM_SHIFT;

/// `MAX_FREQ_COEFFS` as `usize`
const NF: usize = MAX_FREQ_COEFFS;

/// The number of bytes of `SBR_ENV_DATA` after `addHarmonic[MAX_FREQ_COEFFS]` that the copy of
/// `nSfb * sizeof(INT)` bytes in `detectionWithPrediction()` can read
/// (`3 * MAX_FREQ_COEFFS`).
pub const ENV_DATA_TAIL_BYTES: usize = 3 * MAX_FREQ_COEFFS;

/// `THRES_HOLDS`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ThresHolds {
    /// `thresHoldDiff`: threshold for tonality difference
    pub thres_hold_diff: FixpDbl,
    /// `thresHoldDiffGuide`: threshold for tonality difference for the guide
    pub thres_hold_diff_guide: FixpDbl,
    /// `thresHoldTone`: threshold for tonality for a sine
    pub thres_hold_tone: FixpDbl,
    /// `invThresHoldTone`
    pub inv_thres_hold_tone: FixpDbl,
    /// `thresHoldToneGuide`: threshold for tonality for a sine for the guide
    pub thres_hold_tone_guide: FixpDbl,
    /// `sfmThresSbr`: tonality flatness measure threshold for the SBR signal.
    pub sfm_thres_sbr: FixpDbl,
    /// `sfmThresOrig`: tonality flatness measure threshold for the original signal.
    pub sfm_thres_orig: FixpDbl,
    /// `decayGuideOrig`: decay value of the tonality value of the guide for the tone.
    pub decay_guide_orig: FixpDbl,
    /// `decayGuideDiff`: decay value of the tonality value of the guide for the tonality
    /// difference.
    pub decay_guide_diff: FixpDbl,
    /// `derivThresMaxLD64`: threshold for detecting LP character in a signal.
    pub deriv_thres_max_ld_64: FixpDbl,
    /// `derivThresBelowLD64`: threshold for detecting LP character in a signal.
    pub deriv_thres_below_ld_64: FixpDbl,
    /// `derivThresAboveLD64`: threshold for detecting LP character in a signal.
    pub deriv_thres_above_ld_64: FixpDbl,
}

/// `DETECTOR_PARAMETERS_MH`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DetectorParametersMh {
    /// `deltaTime`: maximum allowed transient distance (from frame border in number of qmf
    /// subband sample) for a frame to be considered a transient frame.
    pub delta_time: i32,
    /// `thresHolds`: the thresholds used for detection.
    pub thres_holds: ThresHolds,
    /// `maxComp`: maximum alllowed compensation factor for the envelope data.
    pub max_comp: i32,
}

/// `SBR_MISSING_HARMONICS_DETECTOR` (`HANDLE_SBR_MISSING_HARMONICS_DETECTOR` is
/// `&mut SbrMissingHarmonicsDetector`). The pointer members are owned arrays (see the
/// [module documentation](self)).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SbrMissingHarmonicsDetector {
    /// `qmfNoChannels`
    pub qmf_no_channels: i32,
    /// `nSfb`
    pub n_sfb: i32,
    /// `sampleFreq`
    pub sample_freq: i32,
    /// `previousTransientFlag`
    pub previous_transient_flag: i32,
    /// `previousTransientFrame`
    pub previous_transient_frame: i32,
    /// `previousTransientPos`
    pub previous_transient_pos: i32,

    /// `noVecPerFrame`
    pub no_vec_per_frame: i32,
    /// `transientPosOffset`
    pub transient_pos_offset: i32,

    /// `move`
    pub move_: i32,
    /// `totNoEst`
    pub tot_no_est: i32,
    /// `noEstPerFrame`
    pub no_est_per_frame: i32,
    /// `timeSlots`
    pub time_slots: i32,

    /// `guideScfb` (`Ram_Sbr_guideScfb`)
    pub guide_scfb: [u8; NF],

    /// `prevEnvelopeCompensation` (`Ram_Sbr_prevEnvelopeCompensation`)
    pub prev_envelope_compensation: [u8; NF],

    /// `detectionVectors[i]`: row `i` of `Ram_Sbr_detectionVectors`
    pub detection_vectors: [u8; MAX_NO_OF_ESTIMATES * NF],
    /// `tonalityDiff`
    pub tonality_diff: [[FixpDbl; NF]; MAX_NO_OF_ESTIMATES / 2],
    /// `sfmOrig`
    pub sfm_orig: [[FixpDbl; NF]; MAX_NO_OF_ESTIMATES / 2],
    /// `sfmSbr`
    pub sfm_sbr: [[FixpDbl; NF]; MAX_NO_OF_ESTIMATES / 2],
    /// `mhParams` (`None` for `NULL`)
    pub mh_params: Option<&'static DetectorParametersMh>,
    /// `guideVectors[i].guideVectorDiff`: row `i` of `Ram_Sbr_guideVectorDiff`
    pub guide_vector_diff: [FixpDbl; MAX_NO_OF_ESTIMATES * NF],
    /// `guideVectors[i].guideVectorOrig`: row `i` of `Ram_Sbr_guideVectorOrig`
    pub guide_vector_orig: [FixpDbl; MAX_NO_OF_ESTIMATES * NF],
    /// `guideVectors[i].guideVectorDetected`: row `i` of `Ram_Sbr_guideVectorDetected`
    pub guide_vector_detected: [u8; MAX_NO_OF_ESTIMATES * NF],
}

impl Default for SbrMissingHarmonicsDetector {
    fn default() -> Self {
        SbrMissingHarmonicsDetector {
            qmf_no_channels: 0,
            n_sfb: 0,
            sample_freq: 0,
            previous_transient_flag: 0,
            previous_transient_frame: 0,
            previous_transient_pos: 0,
            no_vec_per_frame: 0,
            transient_pos_offset: 0,
            move_: 0,
            tot_no_est: 0,
            no_est_per_frame: 0,
            time_slots: 0,
            guide_scfb: [0; NF],
            prev_envelope_compensation: [0; NF],
            detection_vectors: [0; MAX_NO_OF_ESTIMATES * NF],
            tonality_diff: [[0; NF]; MAX_NO_OF_ESTIMATES / 2],
            sfm_orig: [[0; NF]; MAX_NO_OF_ESTIMATES / 2],
            sfm_sbr: [[0; NF]; MAX_NO_OF_ESTIMATES / 2],
            mh_params: None,
            guide_vector_diff: [0; MAX_NO_OF_ESTIMATES * NF],
            guide_vector_orig: [0; MAX_NO_OF_ESTIMATES * NF],
            guide_vector_detected: [0; MAX_NO_OF_ESTIMATES * NF],
        }
    }
}

/// The guide vectors (`GUIDE_VECTORS guideVectors[MAX_NO_OF_ESTIMATES]` of C, pointers to
/// rows): mutable views of the three flat arrays.
struct Guides<'a> {
    diff: &'a mut [FixpDbl; MAX_NO_OF_ESTIMATES * NF],
    orig: &'a mut [FixpDbl; MAX_NO_OF_ESTIMATES * NF],
    detected: &'a mut [u8; MAX_NO_OF_ESTIMATES * NF],
}

/// `diff(pTonalityOrig, pDiffMapped2Scfb, pFreqBandTable, nScfb, indexVector)`: the
/// difference in tonality between original and SBR (scaled by RELAXATION). `row` is the row of
/// `pTonalityOrig` in the flat quota matrix.
fn diff(
    quota: &[FixpDbl],
    row: i32,
    p_diff_mapped2_scfb: &mut [FixpDbl],
    p_freq_band_table: &[u8],
    n_scfb: i32,
    index_vector: &[i8],
) {
    let mut scale = 0;

    for i in 0..n_scfb.max(0) as usize {
        let ll = p_freq_band_table[i];
        let lu = p_freq_band_table[i + 1];

        let mut max_val_orig: FixpDbl = 0;
        let mut max_val_sbr: FixpDbl = 0;

        let mut k = ll;
        while k < lu {
            max_val_orig = fix_max(max_val_orig, quota_at(quota, row, k as i32));
            max_val_sbr =
                fix_max(max_val_sbr, quota_at(quota, row, index_vector[k as usize] as i32));
            k = k.wrapping_add(1);
        }

        if max_val_sbr >= RELAXATION {
            let tmp = f_div_norm(max_val_orig, max_val_sbr, &mut scale);
            p_diff_mapped2_scfb[i] = scale_value(
                f_mult(tmp, RELAXATION_FRACT),
                fix_max(-(DFRACT_BITS - 1), scale - RELAXATION_SHIFT),
            );
        } else {
            p_diff_mapped2_scfb[i] = max_val_orig;
        }
    }
}

/// `calculateFlatnessMeasure(pQuotaBuffer, indexVector, pSfmOrigVec, pSfmSbrVec,
/// pFreqBandTable, nSfb)`: a flatness measure of the tonality measures (scaled by 1/4). `row`
/// is the row of `pQuotaBuffer` in the flat quota matrix.
fn calculate_flatness_measure(
    quota: &[FixpDbl],
    row: i32,
    index_vector: &[i8],
    p_sfm_orig_vec: &mut [FixpDbl],
    p_sfm_sbr_vec: &mut [FixpDbl],
    p_freq_band_table: &[u8],
    n_sfb: i32,
) {
    for i in 0..n_sfb.max(0) as usize {
        let ll = p_freq_band_table[i] as i32;
        let lu = p_freq_band_table[i + 1] as i32;
        p_sfm_orig_vec[i] = MAXVAL_DBL >> 2;
        p_sfm_sbr_vec[i] = MAXVAL_DBL >> 2;

        if lu - ll > 1 {
            let inv_bands = get_inv_int(lu - ll);
            let mut shift_fac_sum0 = 0;
            let mut shift_fac_sum1 = 0;
            let mut am_orig: FixpDbl = 0;
            let mut am_transp: FixpDbl = 0;
            let mut gm_orig: FixpDbl = MAXVAL_DBL;
            let mut gm_transp: FixpDbl = MAXVAL_DBL;

            for j in ll..lu {
                let sfm_orig = quota_at(quota, row, j);
                let sfm_transp = quota_at(quota, row, index_vector[j as usize] as i32);

                am_orig = am_orig.wrapping_add(f_mult(sfm_orig, inv_bands));
                am_transp = am_transp.wrapping_add(f_mult(sfm_transp, inv_bands));

                let shift_fac0 = count_leading_bits(sfm_orig);
                let shift_fac1 = count_leading_bits(sfm_transp);

                gm_orig = f_mult(gm_orig, sfm_orig << shift_fac0);
                gm_transp = f_mult(gm_transp, sfm_transp << shift_fac1);

                shift_fac_sum0 += shift_fac0;
                shift_fac_sum1 += shift_fac1;
            }

            if gm_orig > 0 {
                let mut tmp1 = calc_ld_data(gm_orig); // CalcLd64(x)/64
                tmp1 = f_mult(inv_bands, tmp1); // y*CalcLd64(x)/64

                // y*k/64
                let accu = (-shift_fac_sum0).wrapping_shl((DFRACT_BITS - 1 - 8) as u32);
                let mut tmp2 = f_mult_div2(inv_bands, accu) << (2 + 1);

                tmp2 = tmp1.wrapping_add(tmp2); // y*CalcLd64(x)/64 + y*k/64
                gm_orig = calc_inv_ld_data(tmp2); // CalcInvLd64(z');
            } else {
                gm_orig = 0;
            }

            if gm_transp > 0 {
                let mut tmp1 = calc_ld_data(gm_transp); // CalcLd64(x)/64
                tmp1 = f_mult(inv_bands, tmp1); // y*CalcLd64(x)/64

                // y*k/64
                let accu = (-shift_fac_sum1).wrapping_shl((DFRACT_BITS - 1 - 8) as u32);
                let mut tmp2 = f_mult_div2(inv_bands, accu) << (2 + 1);

                tmp2 = tmp1.wrapping_add(tmp2); // y*CalcLd64(x)/64 + y*k/64
                gm_transp = calc_inv_ld_data(tmp2); // CalcInvLd64(z');
            } else {
                gm_transp = 0;
            }
            if am_orig != 0 {
                p_sfm_orig_vec[i] = fdk_sbr_enc_lsi_divide_scale_fract(gm_orig, am_orig, SFM_SCALE);
            }

            if am_transp != 0 {
                p_sfm_sbr_vec[i] =
                    fdk_sbr_enc_lsi_divide_scale_fract(gm_transp, am_transp, SFM_SCALE);
            }
        }
    }
}

/// The rows of the tonality difference and flatness measures of all estimates (the
/// persistent rows of the detector and the scratch rows of the C code).
type EstRows = [[FixpDbl; NF]; MAX_NO_OF_ESTIMATES];

/// `calculateDetectorInput(pQuotaBuffer, indexVector, tonalityDiff, pSfmOrig, pSfmSbr,
/// freqBandTable, nSfb, noEstPerFrame, move)`
#[allow(clippy::too_many_arguments)]
fn calculate_detector_input(
    quota: &[FixpDbl],
    index_vector: &[i8],
    tonality_diff: &mut EstRows,
    p_sfm_orig: &mut EstRows,
    p_sfm_sbr: &mut EstRows,
    freq_band_table: &[u8],
    n_sfb: i32,
    no_est_per_frame: i32,
    move_: i32,
) {
    // New estimate.
    for est in 0..no_est_per_frame.max(0) {
        let e = (est + move_) as usize;
        diff(quota, est + move_, &mut tonality_diff[e], freq_band_table, n_sfb, index_vector);

        calculate_flatness_measure(
            quota,
            est + move_,
            index_vector,
            &mut p_sfm_orig[e],
            &mut p_sfm_sbr[e],
            freq_band_table,
            n_sfb,
        );
    }
}

/// `removeLowPassDetection(pAddHarmSfb, pDetectionVectors, start, stop, nSfb, pFreqBandTable,
/// pNrgVector, mhThresh)`: removes a newly detected missing harmonic that is in fact a low-pass
/// filtered input signal.
#[allow(clippy::too_many_arguments)]
fn remove_low_pass_detection(
    p_add_harm_sfb: &mut [u8],
    p_detection_vectors: &mut [u8],
    start: i32,
    stop: i32,
    n_sfb: i32,
    p_freq_band_table: &[u8],
    p_nrg_vector: &[FixpDbl],
    mh_thresh: &ThresHolds,
) {
    let mut max_deriv_pos = p_freq_band_table[n_sfb as usize] as i32;
    let num_bands = p_freq_band_table[n_sfb as usize] as i32;
    let mut b_lp_signal = 0;

    let mut max_val_ld64 = fl2fxconst_dbl(-1.0);
    let mut i = num_bands - 1 - 2;
    while i > p_freq_band_table[0] as i32 {
        let nrg_low = p_nrg_vector[i as usize];
        let nrg_high = p_nrg_vector[(i + 2) as usize];

        if nrg_low != 0 && nrg_low > nrg_high {
            let nrg_low_ld64 = calc_ld_data(nrg_low >> 1);
            let nrg_diff_ld64 = calc_ld_data((nrg_low >> 1).wrapping_sub(nrg_high >> 1));
            let val_ld64 = nrg_diff_ld64.wrapping_sub(nrg_low_ld64);
            if val_ld64 > max_val_ld64 {
                max_deriv_pos = i;
                max_val_ld64 = val_ld64;
            }
            if max_val_ld64 > mh_thresh.deriv_thres_max_ld_64 {
                break;
            }
        }
        i -= 1;
    }

    // Find the largest "gradient" above. (should be relatively flat, hence we expect a low
    // value if the signal is LP.
    let mut max_val_above_ld64 = fl2fxconst_dbl(-1.0);
    let mut i = num_bands - 1 - 2;
    while i > max_deriv_pos + 2 {
        let nrg_low = p_nrg_vector[i as usize];
        let nrg_high = p_nrg_vector[(i + 2) as usize];

        if nrg_low != 0 && nrg_low > nrg_high {
            let nrg_low_ld64 = calc_ld_data(nrg_low >> 1);
            let nrg_diff_ld64 = calc_ld_data((nrg_low >> 1).wrapping_sub(nrg_high >> 1));
            let val_ld64 = nrg_diff_ld64.wrapping_sub(nrg_low_ld64);
            if val_ld64 > max_val_above_ld64 {
                max_val_above_ld64 = val_ld64;
            }
        } else if nrg_high != 0 && nrg_high > nrg_low {
            let nrg_high_ld64 = calc_ld_data(nrg_high >> 1);
            let nrg_diff_ld64 = calc_ld_data((nrg_high >> 1).wrapping_sub(nrg_low >> 1));
            let val_ld64 = nrg_diff_ld64.wrapping_sub(nrg_high_ld64);
            if val_ld64 > max_val_above_ld64 {
                max_val_above_ld64 = val_ld64;
            }
        }
        i -= 1;
    }

    if max_val_ld64 > mh_thresh.deriv_thres_max_ld_64
        && max_val_above_ld64 < mh_thresh.deriv_thres_above_ld_64
    {
        b_lp_signal = 1;

        let mut i = max_deriv_pos - 1;
        while i > max_deriv_pos - 5 && i >= 0 {
            let at = p_nrg_vector[(max_deriv_pos + 2) as usize];
            if p_nrg_vector[i as usize] != 0 && p_nrg_vector[i as usize] > at {
                let nrg_diff_ld64 =
                    calc_ld_data((p_nrg_vector[i as usize] >> 1).wrapping_sub(at >> 1));
                let nrg_ld64 = calc_ld_data(p_nrg_vector[i as usize] >> 1);
                let val_ld64 = nrg_diff_ld64.wrapping_sub(nrg_ld64);
                if val_ld64 < mh_thresh.deriv_thres_below_ld_64 {
                    b_lp_signal = 0;
                    break;
                }
            } else {
                b_lp_signal = 0;
                break;
            }
            i -= 1;
        }
    }

    if b_lp_signal != 0 {
        let mut i = 0usize;
        while i < n_sfb as usize {
            if max_deriv_pos >= p_freq_band_table[i] as i32
                && max_deriv_pos < p_freq_band_table[i + 1] as i32
            {
                break;
            }
            i += 1;
        }

        if p_add_harm_sfb[i] != 0 {
            p_add_harm_sfb[i] = 0;
            for est in start..stop {
                p_detection_vectors[est as usize * NF + i] = 0;
            }
        }
    }
}

/// `isDetectionOfNewToneAllowed(pFrameInfo, pDetectionStartPos, noEstPerFrame,
/// prevTransientFrame, prevTransientPos, prevTransientFlag, transientPosOffset,
/// transientFlag, transientPos, deltaTime, h_sbrMissingHarmonicsDetector)`
#[allow(clippy::too_many_arguments)]
fn is_detection_of_new_tone_allowed(
    p_frame_info: &SbrFrameInfo,
    p_detection_start_pos: &mut i32,
    no_est_per_frame: i32,
    prev_transient_frame: i32,
    prev_transient_pos: i32,
    prev_transient_flag: i32,
    transient_pos_offset: i32,
    transient_flag: i32,
    transient_pos: i32,
    delta_time: i32,
    h: &mut SbrMissingHarmonicsDetector,
) -> i32 {
    // Determine if this is a frame where a transient starts...
    // If the transient flag was set the previous frame but not the transient frame flag, the
    // transient frame flag is set in the current frame.
    let mut transient_frame = 0;
    if transient_flag != 0 {
        if transient_pos + transient_pos_offset
            < p_frame_info.borders[p_frame_info.n_envelopes as usize]
        {
            transient_frame = 1;
            if no_est_per_frame > 1 {
                if transient_pos + transient_pos_offset > h.time_slots >> 1 {
                    *p_detection_start_pos = no_est_per_frame;
                } else {
                    *p_detection_start_pos = no_est_per_frame >> 1;
                }
            } else {
                *p_detection_start_pos = no_est_per_frame;
            }
        }
    } else if prev_transient_flag != 0 && prev_transient_frame == 0 {
        transient_frame = 1;
        *p_detection_start_pos = 0;
    }

    // Determine if detection of new missing harmonics are allowed. If the frame contains a
    // transient it's ok. If the previous frame contained a transient it needs to be
    // sufficiently close to the start of the current frame.
    let mut new_detection_allowed = 0;
    if transient_frame != 0 {
        new_detection_allowed = 1;
    } else if prev_transient_frame != 0
        && (p_frame_info.borders[0] - (prev_transient_pos + transient_pos_offset - h.time_slots))
            .abs()
            < delta_time
    {
        new_detection_allowed = 1;
        *p_detection_start_pos = 0;
    }

    h.previous_transient_flag = transient_flag;
    h.previous_transient_frame = transient_frame;
    h.previous_transient_pos = transient_pos;

    new_detection_allowed
}

/// `transientCleanUp(quotaBuffer, nSfb, detectionVectors, pAddHarmSfb, pPrevAddHarmSfb,
/// signBuffer, pFreqBandTable, start, stop, newDetectionAllowed, pNrgVector, mhThresh)`
#[allow(clippy::too_many_arguments)]
fn transient_clean_up(
    quota: &[FixpDbl],
    n_sfb: i32,
    detection_vectors: &mut [u8],
    p_add_harm_sfb: &mut [u8],
    p_prev_add_harm_sfb: &[u8],
    sign: &[i32],
    p_freq_band_table: &[u8],
    start: i32,
    stop: i32,
    new_detection_allowed: i32,
    p_nrg_vector: &[FixpDbl],
    mh_thresh: &ThresHolds,
) {
    let nsfb = n_sfb.max(0) as usize;
    for est in start..stop {
        for i in 0..nsfb {
            p_add_harm_sfb[i] =
                (p_add_harm_sfb[i] != 0 || detection_vectors[est as usize * NF + i] != 0) as u8;
        }
    }

    if new_detection_allowed == 1 {
        // Check for duplication of sines located on the border of two scf-bands.
        for i in 0..nsfb.saturating_sub(1) {
            // detection in adjacent channels.
            if p_add_harm_sfb[i] != 0 && p_add_harm_sfb[i + 1] != 0 {
                let mut li = p_freq_band_table[i] as i32;
                let mut ui = p_freq_band_table[i + 1] as i32;

                // Find maximum tonality in the the two scf bands.
                let mut max_pos_time1 = start;
                let mut max_pos1 = li;
                let mut max_val1 = quota_at(quota, start, li);
                for est in start..stop {
                    for j in li..ui {
                        let q = quota_at(quota, est, j);
                        if q > max_val1 {
                            max_val1 = q;
                            max_pos1 = j;
                            max_pos_time1 = est;
                        }
                    }
                }

                li = p_freq_band_table[i + 1] as i32;
                ui = p_freq_band_table[i + 2] as i32;

                // Find maximum tonality in the the two scf bands.
                let mut max_pos_time2 = start;
                let mut max_pos2 = li;
                let mut max_val2 = quota_at(quota, start, li);
                for est in start..stop {
                    for j in li..ui {
                        let q = quota_at(quota, est, j);
                        if q > max_val2 {
                            max_val2 = q;
                            max_pos2 = j;
                            max_pos_time2 = est;
                        }
                    }
                }

                // If the maximum values are in adjacent QMF-channels, we need to remove the
                // lowest of the two.
                if max_pos2 - max_pos1 < 2 {
                    if p_prev_add_harm_sfb[i] == 1 && p_prev_add_harm_sfb[i + 1] == 0 {
                        // Keep the lower, remove the upper.
                        p_add_harm_sfb[i + 1] = 0;
                        for est in start..stop {
                            detection_vectors[est as usize * NF + i + 1] = 0;
                        }
                    } else if p_prev_add_harm_sfb[i] == 0 && p_prev_add_harm_sfb[i + 1] == 1 {
                        // Keep the upper, remove the lower.
                        p_add_harm_sfb[i] = 0;
                        for est in start..stop {
                            detection_vectors[est as usize * NF + i] = 0;
                        }
                    } else {
                        // If the maximum values are in adjacent QMF-channels, and if the signs
                        // indicate that it is the same sine, we need to remove the lowest of
                        // the two.
                        if max_val1 > max_val2 {
                            if sign_at(sign, max_pos_time1, max_pos2) < 0
                                && sign_at(sign, max_pos_time1, max_pos1) > 0
                            {
                                // Keep the lower, remove the upper.
                                p_add_harm_sfb[i + 1] = 0;
                                for est in start..stop {
                                    detection_vectors[est as usize * NF + i + 1] = 0;
                                }
                            }
                        } else if sign_at(sign, max_pos_time2, max_pos2) < 0
                            && sign_at(sign, max_pos_time2, max_pos1) > 0
                        {
                            // Keep the upper, remove the lower.
                            p_add_harm_sfb[i] = 0;
                            for est in start..stop {
                                detection_vectors[est as usize * NF + i] = 0;
                            }
                        }
                    }
                }
            }
        }

        // Make sure that the detection is not the cut-off of a low pass filter.
        remove_low_pass_detection(
            p_add_harm_sfb,
            detection_vectors,
            start,
            stop,
            n_sfb,
            p_freq_band_table,
            p_nrg_vector,
            mh_thresh,
        );
    } else {
        // If a missing harmonic wasn't missing the previous frame the transient-flag needs to
        // be set in order to be allowed to detect it.
        for i in 0..nsfb {
            if p_add_harm_sfb[i] as i32 - p_prev_add_harm_sfb[i] as i32 > 0 {
                p_add_harm_sfb[i] = 0;
            }
        }
    }
}

/// `signBuffer[j][i]`: the sign matrix has the layout of the quota matrix.
#[inline]
fn sign_at(sign: &[i32], j: i32, i: i32) -> i32 {
    let idx = j as isize * 64 + i as isize;
    if idx < 0 {
        0
    } else {
        sign.get(idx as usize).copied().unwrap_or(0)
    }
}

/// `detection(quotaBuffer, pDiffVecScfb, nSfb, pHarmVec, pFreqBandTable, sfmOrig, sfmSbr,
/// guideVectors, newGuideVectors, mhThresh)`: the detection for one tonality estimate.
/// `est` is the estimate (the row of `quotaBuffer`, `pHarmVec` and the guide vectors), `new`
/// the row of `newGuideVectors` (`est + 1`, or `est` for the last estimate).
#[allow(clippy::too_many_arguments)]
fn detection(
    quota: &[FixpDbl],
    est: usize,
    p_diff_vec_scfb: &[FixpDbl],
    n_sfb: i32,
    detection_vectors: &mut [u8],
    p_freq_band_table: &[u8],
    sfm_orig: &[FixpDbl],
    sfm_sbr: &[FixpDbl],
    guides: &mut Guides,
    new: usize,
    mh_thresh: &ThresHolds,
) {
    let nsfb = n_sfb.max(0) as usize;
    let g = est * NF;
    let ng = new * NF;
    let p_harm_vec = &mut detection_vectors[est * NF..est * NF + NF];

    // Do detection on the difference vector, i.e. the difference between the original and the
    // transposed.
    for i in 0..nsfb {
        let mut thres_temp = if guides.diff[g + i] != 0 {
            f_max(
                f_mult(mh_thresh.decay_guide_diff, guides.diff[g + i]),
                mh_thresh.thres_hold_diff_guide,
            )
        } else {
            mh_thresh.thres_hold_diff
        };

        thres_temp = f_min(thres_temp, mh_thresh.thres_hold_diff);

        if p_diff_vec_scfb[i] > thres_temp {
            p_harm_vec[i] = 1;
            guides.diff[ng + i] = p_diff_vec_scfb[i];
        } else {
            // If the guide wasn't zero, but the current level is to low, start tracking the
            // decay on the tone in the original rather than the difference.
            if guides.diff[g + i] != 0 {
                guides.orig[g + i] = mh_thresh.thres_hold_tone_guide;
            }
        }
    }

    // Trace tones in the original signal that at one point have been detected because they
    // will be replaced by multiple tones in the sbr signal.

    for i in 0..nsfb {
        let ll = p_freq_band_table[i] as i32;
        let lu = p_freq_band_table[i + 1] as i32;

        let mut thres_orig = fix_max(
            f_mult(guides.orig[g + i], mh_thresh.decay_guide_orig),
            mh_thresh.thres_hold_tone_guide,
        );
        thres_orig = fix_min(thres_orig, mh_thresh.thres_hold_tone);

        if guides.orig[g + i] != 0 {
            for j in ll..lu {
                let q = quota_at(quota, est as i32, j);
                if q > thres_orig {
                    p_harm_vec[i] = 1;
                    guides.orig[ng + i] = q;
                }
            }
        }
    }

    // Check for multiple sines in the transposed signal, where there is only one in the
    // original.
    let thres_orig = mh_thresh.thres_hold_tone;

    for i in 0..nsfb {
        let ll = p_freq_band_table[i] as i32;
        let lu = p_freq_band_table[i + 1] as i32;

        if p_harm_vec[i] == 0 {
            if lu - ll > 1 {
                for j in ll..lu {
                    let q = quota_at(quota, est as i32, j);
                    if q > thres_orig
                        && (sfm_sbr[i] > mh_thresh.sfm_thres_sbr
                            && sfm_orig[i] < mh_thresh.sfm_thres_orig)
                    {
                        p_harm_vec[i] = 1;
                        guides.orig[ng + i] = q;
                    }
                }
            } else if i + 1 < nsfb {
                let ll = p_freq_band_table[i] as i32;
                let q = quota_at(quota, est as i32, ll);

                if i > 0 {
                    if q > mh_thresh.thres_hold_tone
                        && (p_diff_vec_scfb[i + 1] < mh_thresh.inv_thres_hold_tone
                            || p_diff_vec_scfb[i - 1] < mh_thresh.inv_thres_hold_tone)
                    {
                        p_harm_vec[i] = 1;
                        guides.orig[ng + i] = q;
                    }
                } else if q > mh_thresh.thres_hold_tone
                    && p_diff_vec_scfb[i + 1] < mh_thresh.inv_thres_hold_tone
                {
                    p_harm_vec[i] = 1;
                    guides.orig[ng + i] = q;
                }
            }
        }
    }
}

/// `detectionWithPrediction(quotaBuffer, pDiffVecScfb, signBuffer, nSfb, pFreqBandTable,
/// sfmOrig, sfmSbr, detectionVectors, pPrevAddHarmSfb, guideVectors, noEstPerFrame,
/// detectionStart, totNoEst, newDetectionAllowed, pAddHarmFlag, pAddHarmSfb, pNrgVector,
/// mhParams)`: the detection for every tonality estimate, using forward prediction.
///
/// `env_data_tail` is the image of the members of `SBR_ENV_DATA` after `addHarmonic` (see the
/// [module documentation](self)).
#[allow(clippy::too_many_arguments)]
fn detection_with_prediction(
    quota: &[FixpDbl],
    p_diff_vec_scfb: &EstRows,
    sign: &[i32],
    n_sfb: i32,
    p_freq_band_table: &[u8],
    sfm_orig: &EstRows,
    sfm_sbr: &EstRows,
    detection_vectors: &mut [u8; MAX_NO_OF_ESTIMATES * NF],
    p_prev_add_harm_sfb: &mut [u8; NF],
    guides: &mut Guides,
    no_est_per_frame: i32,
    detection_start: i32,
    tot_no_est: i32,
    new_detection_allowed: i32,
    p_add_harm_flag: &mut i32,
    p_add_harm_sfb: &mut [u8],
    p_nrg_vector: &[FixpDbl],
    mh_params: &DetectorParametersMh,
    env_data_tail: &[u8; ENV_DATA_TAIL_BYTES],
) {
    let start;
    let nsfb = n_sfb.max(0) as usize;
    let _ = no_est_per_frame;

    p_add_harm_sfb[..nsfb].fill(0);

    if new_detection_allowed != 0 {
        // Since we don't want to use the transient region for detection (since the tonality
        // values tend to be a bit unreliable for this region) the guide-values are copied to
        // the current starting point.
        if tot_no_est > 1 {
            start = detection_start + 1;

            if start != 0 {
                let s = start as usize * NF;
                guides.diff.copy_within(0..nsfb, s);
                guides.orig.copy_within(0..nsfb, s);
                let p = (start - 1) as usize * NF;
                guides.detected[p..p + nsfb].fill(0);
            }
        } else {
            start = 0;
        }
    } else {
        start = 0;
    }

    for est in start..tot_no_est {
        let e = est as usize;
        // Do detection on the current frame using guide-info from the previous.
        if est > 0 {
            guides.detected[e * NF..e * NF + nsfb]
                .copy_from_slice(&detection_vectors[(e - 1) * NF..(e - 1) * NF + nsfb]);
        }

        detection_vectors[e * NF..e * NF + nsfb].fill(0);

        if est < tot_no_est - 1 {
            let n = (e + 1) * NF;
            guides.diff[n..n + nsfb].fill(0);
            guides.orig[n..n + nsfb].fill(0);
            guides.detected[n..n + nsfb].fill(0);

            detection(
                quota,
                e,
                &p_diff_vec_scfb[e],
                n_sfb,
                detection_vectors,
                p_freq_band_table,
                &sfm_orig[e],
                &sfm_sbr[e],
                guides,
                e + 1,
                &mh_params.thres_holds,
            );
        } else {
            let n = e * NF;
            guides.diff[n..n + nsfb].fill(0);
            guides.orig[n..n + nsfb].fill(0);
            guides.detected[n..n + nsfb].fill(0);

            detection(
                quota,
                e,
                &p_diff_vec_scfb[e],
                n_sfb,
                detection_vectors,
                p_freq_band_table,
                &sfm_orig[e],
                &sfm_sbr[e],
                guides,
                e,
                &mh_params.thres_holds,
            );
        }
    }

    // Clean up the detection.
    transient_clean_up(
        quota,
        n_sfb,
        detection_vectors,
        p_add_harm_sfb,
        p_prev_add_harm_sfb,
        sign,
        p_freq_band_table,
        start,
        tot_no_est,
        new_detection_allowed,
        p_nrg_vector,
        &mh_params.thres_holds,
    );

    // Set flag...
    *p_add_harm_flag = 0;
    for &a in p_add_harm_sfb.iter().take(nsfb) {
        if a != 0 {
            *p_add_harm_flag = 1;
            break;
        }
    }

    p_prev_add_harm_sfb[..nsfb].copy_from_slice(&p_add_harm_sfb[..nsfb]);

    // C: FDKmemcpy(guideVectors[0].guideVectorDetected, pAddHarmSfb, nSfb * sizeof(INT))
    // copies 4 * nSfb bytes: addHarmonic[MAX_FREQ_COEFFS] and the members of SBR_ENV_DATA that
    // follow it, into the guide vectors 0..3 (one allocation).
    let n = (4 * nsfb).min(MAX_FREQ_COEFFS + ENV_DATA_TAIL_BYTES).min(guides.detected.len());
    for b in 0..n {
        guides.detected[b] = if b < NF { p_add_harm_sfb[b] } else { env_data_tail[b - NF] };
    }

    for i in 0..nsfb {
        guides.diff[i] = 0;
        guides.orig[i] = 0;

        if p_add_harm_sfb[i] == 1 {
            // If we had a detection use the guide-value in the next frame from the last
            // estimate were the detection was done.
            for est in start..tot_no_est {
                let e = est as usize * NF;
                if guides.diff[e + i] != 0 {
                    guides.diff[i] = guides.diff[e + i];
                }
                if guides.orig[e + i] != 0 {
                    guides.orig[i] = guides.orig[e + i];
                }
            }
        }
    }
}

/// `calculateCompVector(pAddHarmSfb, pTonalityMatrix, pSignMatrix, pEnvComp, nSfb,
/// freqBandTable, totNoEst, maxComp, pPrevEnvComp, newDetectionAllowed)`: the compensation of
/// the envelope data for a sine between two QMF bands.
#[allow(clippy::too_many_arguments)]
fn calculate_comp_vector(
    p_add_harm_sfb: &[u8],
    quota: &[FixpDbl],
    sign: &[i32],
    p_env_comp: &mut [u8],
    n_sfb: i32,
    freq_band_table: &[u8],
    tot_no_est: i32,
    max_comp: i32,
    p_prev_env_comp: &mut [u8; NF],
    new_detection_allowed: i32,
) {
    let nsfb = n_sfb.max(0) as usize;
    p_env_comp[..nsfb].fill(0);

    for scf_band in 0..nsfb {
        if p_add_harm_sfb[scf_band] != 0 {
            // A missing sine was detected
            let ll = freq_band_table[scf_band] as i32;
            let lu = freq_band_table[scf_band + 1] as i32;

            let mut max_pos_f = 0; // First find the maximum
            let mut max_pos_t = 0;
            let mut max_val: FixpDbl = 0;

            for est in 0..tot_no_est {
                for l in ll..lu {
                    let q = quota_at(quota, est, l);
                    if q > max_val {
                        max_val = q;
                        max_pos_f = l;
                        max_pos_t = est;
                    }
                }
            }

            // If the maximum tonality is at the lower border of the scalefactor band, we check
            // the sign of the adjacent channels to see if this sine is shared by the lower
            // channel. If so, the energy of the single sine will be present in two scalefactor
            // bands in the SBR data, which will cause problems in the decoder, when we add a
            // sine to just one of the channels.
            if max_pos_f == ll && scf_band != 0 && p_add_harm_sfb[scf_band - 1] == 0 {
                // No detection below
                if sign_at(sign, max_pos_t, max_pos_f - 1) > 0
                    && sign_at(sign, max_pos_t, max_pos_f) < 0
                {
                    // The comp value is calulated as the tonallity value, i.e we want to
                    // reduce the envelope data for this channel with as much as the tonality
                    // that is spread from the channel above. (ld64(RELAXATION) =
                    // 0.31143075889)
                    let mut tmp = fixp_abs(
                        calc_ld_data(quota_at(quota, max_pos_t, max_pos_f - 1))
                            .wrapping_add(RELAXATION_LD64),
                    );
                    tmp = (tmp >> (DFRACT_BITS - 1 - LD_DATA_SHIFT - 1)).wrapping_add(1); // shift one bit less for rounding
                    let mut comp_value = tmp >> 1;

                    // limit the comp-value
                    if comp_value > max_comp {
                        comp_value = max_comp;
                    }

                    p_env_comp[scf_band - 1] = comp_value as u8;
                }
            }

            // Same as above, but for the upper end of the scalefactor-band.
            if max_pos_f == lu - 1
                && scf_band + 1 < nsfb
                && p_add_harm_sfb[scf_band + 1] == 0
                && sign_at(sign, max_pos_t, max_pos_f) > 0
                && sign_at(sign, max_pos_t, max_pos_f + 1) < 0
            {
                // Upper border
                let mut tmp = fixp_abs(
                    calc_ld_data(quota_at(quota, max_pos_t, max_pos_f + 1))
                        .wrapping_add(RELAXATION_LD64),
                );
                tmp = (tmp >> (DFRACT_BITS - 1 - LD_DATA_SHIFT - 1)).wrapping_add(1); // shift one bit less for rounding
                let mut comp_value = tmp >> 1;

                if comp_value > max_comp {
                    comp_value = max_comp;
                }

                p_env_comp[scf_band + 1] = comp_value as u8;
            }
        }
    }

    if new_detection_allowed == 0 {
        for scf_band in 0..nsfb {
            if p_env_comp[scf_band] != 0 && p_prev_env_comp[scf_band] == 0 {
                p_env_comp[scf_band] = 0;
            }
        }
    }

    // remember the value for the next frame.
    p_prev_env_comp[..nsfb].copy_from_slice(&p_env_comp[..nsfb]);
}

/// `FDKsbrEnc_SbrMissingHarmonicsDetectorQmf(h_sbrMHDet, pQuotaBuffer, pSignBuffer,
/// indexVector, pFrameInfo, pTranInfo, pAddHarmonicsFlag, pAddHarmonicsScaleFactorBands,
/// freqBandTable, nSfb, envelopeCompensation, pNrgVector)`: detects where strong tonal
/// components will be missing after HFR in the decoder.
///
/// `p_quota_buffer` and `p_sign_buffer` are the flat quota and sign matrices (rows of 64
/// values); `env_data_tail` is the image of the members of `SBR_ENV_DATA` that follow
/// `pAddHarmonicsScaleFactorBands` (see the [module documentation](self)).
#[allow(clippy::too_many_arguments)]
pub fn fdk_sbr_enc_sbr_missing_harmonics_detector_qmf(
    h: &mut SbrMissingHarmonicsDetector,
    p_quota_buffer: &[FixpDbl],
    p_sign_buffer: &[i32],
    index_vector: &[i8],
    p_frame_info: &SbrFrameInfo,
    p_tran_info: &[u8],
    p_add_harmonics_flag: &mut i32,
    p_add_harmonics_scale_factor_bands: &mut [u8],
    freq_band_table: &[u8],
    n_sfb: i32,
    envelope_compensation: &mut [u8],
    p_nrg_vector: &[FixpDbl],
    env_data_tail: &[u8; ENV_DATA_TAIL_BYTES],
) {
    let transient_flag = p_tran_info[1] as i32;
    let transient_pos = p_tran_info[0] as i32;
    let mut transient_det_start = 0;
    let move_ = h.move_;
    let no_est_per_frame = h.no_est_per_frame;
    let tot_no_est = h.tot_no_est;
    let prev_transient_flag = h.previous_transient_flag;
    let prev_transient_frame = h.previous_transient_frame;
    let transient_pos_offset = h.transient_pos_offset;
    let prev_transient_pos = h.previous_transient_pos;
    let Some(mh_params) = h.mh_params else {
        return;
    };
    let delta_time = mh_params.delta_time;
    let max_comp = mh_params.max_comp;

    // Buffer values.
    fdk_assert!(move_ <= (MAX_NO_OF_ESTIMATES >> 1) as i32);
    fdk_assert!(no_est_per_frame <= (MAX_NO_OF_ESTIMATES >> 1) as i32);

    // The persistent rows (0, 1) and the scratch rows (2, 3) of C.
    let mut sfm_sbr: EstRows = [[0; NF]; MAX_NO_OF_ESTIMATES];
    let mut sfm_orig: EstRows = [[0; NF]; MAX_NO_OF_ESTIMATES];
    let mut tonality_diff: EstRows = [[0; NF]; MAX_NO_OF_ESTIMATES];
    sfm_sbr[..(MAX_NO_OF_ESTIMATES / 2)].copy_from_slice(&h.sfm_sbr[..(MAX_NO_OF_ESTIMATES / 2)]);
    sfm_orig[..(MAX_NO_OF_ESTIMATES / 2)].copy_from_slice(&h.sfm_orig[..(MAX_NO_OF_ESTIMATES / 2)]);
    tonality_diff[..(MAX_NO_OF_ESTIMATES / 2)]
        .copy_from_slice(&h.tonality_diff[..(MAX_NO_OF_ESTIMATES / 2)]);

    // Determine if we're allowed to detect "missing harmonics" that wasn't detected before. In
    // order to be allowed to do new detection, there must be a transient in the current
    // frame, or a transient in the previous frame sufficiently close to the current frame.
    let new_detection_allowed = is_detection_of_new_tone_allowed(
        p_frame_info,
        &mut transient_det_start,
        no_est_per_frame,
        prev_transient_frame,
        prev_transient_pos,
        prev_transient_flag,
        transient_pos_offset,
        transient_flag,
        transient_pos,
        delta_time,
        h,
    );

    // Calulate the variables that will be used subsequently for the actual detection
    calculate_detector_input(
        p_quota_buffer,
        index_vector,
        &mut tonality_diff,
        &mut sfm_orig,
        &mut sfm_sbr,
        freq_band_table,
        n_sfb,
        no_est_per_frame,
        move_,
    );

    // Do the actual detection using information from previous detections
    {
        let SbrMissingHarmonicsDetector {
            detection_vectors,
            guide_scfb,
            guide_vector_diff,
            guide_vector_orig,
            guide_vector_detected,
            ..
        } = h;
        let mut guides = Guides {
            diff: guide_vector_diff,
            orig: guide_vector_orig,
            detected: guide_vector_detected,
        };
        detection_with_prediction(
            p_quota_buffer,
            &tonality_diff,
            p_sign_buffer,
            n_sfb,
            freq_band_table,
            &sfm_orig,
            &sfm_sbr,
            detection_vectors,
            guide_scfb,
            &mut guides,
            no_est_per_frame,
            transient_det_start,
            tot_no_est,
            new_detection_allowed,
            p_add_harmonics_flag,
            p_add_harmonics_scale_factor_bands,
            p_nrg_vector,
            mh_params,
            env_data_tail,
        );
    }

    // Calculate the comp vector, so that the energy can be compensated for a sine between two
    // QMF-bands.
    calculate_comp_vector(
        p_add_harmonics_scale_factor_bands,
        p_quota_buffer,
        p_sign_buffer,
        envelope_compensation,
        n_sfb,
        freq_band_table,
        tot_no_est,
        max_comp,
        &mut h.prev_envelope_compensation,
        new_detection_allowed,
    );

    for est in 0..move_.max(0) as usize {
        let src = est + no_est_per_frame as usize;
        tonality_diff[est] = tonality_diff[src];
        sfm_orig[est] = sfm_orig[src];
        sfm_sbr[est] = sfm_sbr[src];
    }
    h.sfm_sbr[..(MAX_NO_OF_ESTIMATES / 2)].copy_from_slice(&sfm_sbr[..(MAX_NO_OF_ESTIMATES / 2)]);
    h.sfm_orig[..(MAX_NO_OF_ESTIMATES / 2)].copy_from_slice(&sfm_orig[..(MAX_NO_OF_ESTIMATES / 2)]);
    h.tonality_diff[..(MAX_NO_OF_ESTIMATES / 2)]
        .copy_from_slice(&tonality_diff[..(MAX_NO_OF_ESTIMATES / 2)]);
}

/// `FDKsbrEnc_CreateSbrMissingHarmonicsDetector(hSbrMHDet, chan)`: clears the detector (the
/// memory of the vectors is owned by the detector). Returns 0.
pub fn fdk_sbr_enc_create_sbr_missing_harmonics_detector(
    h_sbr_mh_det: &mut SbrMissingHarmonicsDetector,
    _chan: i32,
) -> i32 {
    // C clears the structure and points it to newly allocated (zeroed) vectors.
    *h_sbr_mh_det = SbrMissingHarmonicsDetector::default();
    0
}

/// `FDKsbrEnc_InitSbrMissingHarmonicsDetector(hSbrMHDet, sampleFreq, frameSize, nSfb,
/// qmfNoChannels, totNoEst, move, noEstPerFrame, sbrSyntaxFlags)`: returns 0 if OK.
#[allow(clippy::too_many_arguments)]
pub fn fdk_sbr_enc_init_sbr_missing_harmonics_detector(
    hs: &mut SbrMissingHarmonicsDetector,
    sample_freq: i32,
    frame_size: i32,
    n_sfb: i32,
    qmf_no_channels: i32,
    tot_no_est: i32,
    move_: i32,
    no_est_per_frame: i32,
    sbr_syntax_flags: u32,
) -> i32 {
    fdk_assert!(tot_no_est as usize <= MAX_NO_OF_ESTIMATES);

    if sbr_syntax_flags & SBR_SYNTAX_LOW_DELAY != 0 {
        match frame_size {
            1024 | 512 => {
                hs.transient_pos_offset = FRAME_MIDDLE_SLOT_512LD;
                hs.time_slots = 16;
            }
            960 | 480 => {
                hs.transient_pos_offset = FRAME_MIDDLE_SLOT_512LD;
                hs.time_slots = 15;
            }
            _ => return -1,
        }
    } else {
        match frame_size {
            2048 | 1024 => {
                hs.transient_pos_offset = FRAME_MIDDLE_SLOT_2048;
                hs.time_slots = NUMBER_TIME_SLOTS_2048;
            }
            1920 | 960 => {
                hs.transient_pos_offset = crate::fram_gen::FRAME_MIDDLE_SLOT_1920;
                hs.time_slots = NUMBER_TIME_SLOTS_1920;
            }
            _ => return -1,
        }
    }

    if sbr_syntax_flags & SBR_SYNTAX_LOW_DELAY != 0 {
        hs.mh_params = Some(&PARAMS_AAC_LD);
    } else {
        hs.mh_params = Some(&PARAMS_AAC);
    }

    hs.qmf_no_channels = qmf_no_channels;
    hs.sample_freq = sample_freq;
    hs.n_sfb = n_sfb;

    hs.tot_no_est = tot_no_est;
    hs.move_ = move_;
    hs.no_est_per_frame = no_est_per_frame;

    for i in 0..(tot_no_est.max(0) as usize).min(MAX_NO_OF_ESTIMATES) {
        hs.guide_vector_diff[i * NF..i * NF + NF].fill(0);
        hs.guide_vector_orig[i * NF..i * NF + NF].fill(0);
        hs.detection_vectors[i * NF..i * NF + NF].fill(0);
        hs.guide_vector_detected[i * NF..i * NF + NF].fill(0);
    }

    // for(i=0; i<totNoEst/2; i++) {
    for i in 0..MAX_NO_OF_ESTIMATES / 2 {
        hs.tonality_diff[i] = [0; NF];
        hs.sfm_orig[i] = [0; NF];
        hs.sfm_sbr[i] = [0; NF];
    }

    hs.prev_envelope_compensation = [0; NF];
    hs.guide_scfb = [0; NF];
    hs.previous_transient_flag = 0;
    hs.previous_transient_frame = 0;
    hs.previous_transient_pos = 0;

    0
}

/// `FDKsbrEnc_DeleteSbrMissingHarmonicsDetector(hSbrMHDet)`: nothing to free (owned memory).
pub fn fdk_sbr_enc_delete_sbr_missing_harmonics_detector(_h: &mut SbrMissingHarmonicsDetector) {}

/// Moves the first `n_sfb_prev` values of `v` to the end of the first `n_sfb` values (padding
/// with zeros at the start) or keeps the last `n_sfb` of them, as
/// `FDKsbrEnc_ResetSbrMissingHarmonicsDetector()` does for each vector.
fn reset_shift<T: Copy + Default>(v: &mut [T], n_sfb: i32, n_sfb_prev: i32) {
    let mut temp = [T::default(); NF];
    let np = (n_sfb_prev.max(0) as usize).min(NF);
    temp[..np].copy_from_slice(&v[..np]);

    if n_sfb > n_sfb_prev {
        let d = (n_sfb - n_sfb_prev) as usize;
        for x in v.iter_mut().take(d) {
            *x = T::default();
        }
        v[d..(np + d)].copy_from_slice(&temp[..np]);
    } else {
        let d = (n_sfb_prev - n_sfb) as usize;
        v[..(n_sfb.max(0) as usize)].copy_from_slice(&temp[d..(n_sfb.max(0) as usize + d)]);
    }
}

/// `FDKsbrEnc_ResetSbrMissingHarmonicsDetector(hSbrMissingHarmonicsDetector, nSfb)`: returns 0.
pub fn fdk_sbr_enc_reset_sbr_missing_harmonics_detector(
    h: &mut SbrMissingHarmonicsDetector,
    n_sfb: i32,
) -> i32 {
    let n_sfb_prev = h.n_sfb;
    h.n_sfb = n_sfb;

    reset_shift(&mut h.guide_scfb, n_sfb, n_sfb_prev);
    reset_shift(&mut h.guide_vector_diff[..NF], n_sfb, n_sfb_prev);
    reset_shift(&mut h.guide_vector_orig[..NF], n_sfb, n_sfb_prev);
    reset_shift(&mut h.guide_vector_detected[..NF], n_sfb, n_sfb_prev);
    reset_shift(&mut h.prev_envelope_compensation, n_sfb, n_sfb_prev);

    0
}
