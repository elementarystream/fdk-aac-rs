// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libSBRenc/src/tran_det.h, libSBRenc/src/tran_det.cpp.

//! Port of `tran_det.h` and `tran_det.cpp`: the transient detector, the fast transient
//! detector of low delay SBR, and the frame splitter.
//!
//! The energies (`FIXP_DBL **Energies` in C) are passed as rows of 64 values (`&[&[FixpDbl]]`),
//! the envelope estimator's `YBuffer` ([`crate::env_est`]).
//!
//! # Undefined behavior
//!
//! Additions of energies and transient candidates can overflow for extreme input; both builds
//! use 32 bit wrapping instructions, so the port uses `wrapping_*`. Shift amounts that can be
//! out of range for extreme scale factors are taken modulo 32 (`wrapping_shr`), as both builds
//! shift with a register amount.

use fdk_aac_rs_fdk::common_fix::*;
use fdk_aac_rs_fdk::fdk_assert;
use fdk_aac_rs_fdk::scale::{scale_value, scale_value_saturate};

use crate::fram_gen::NUMBER_TIME_SLOTS_2304;
use crate::sbr_def::{MAX_FREQ_COEFFS, QMF_SCALE_OFFSET};
use crate::sbr_encoder::{SbrConfiguration, SBR_SYNTAX_LOW_DELAY};

/// `SBR_TRANSIENT_DETECTOR`
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SbrTransientDetector {
    /// `transients`
    pub transients: [FixpDbl; 32 + (32 / 2)],
    /// `thresholds`
    pub thresholds: [FixpDbl; 64],
    /// `tran_thr`: Master threshold for transient signals
    pub tran_thr: FixpDbl,
    /// `split_thr_m`: Threshold for splitting FIXFIX-frames into 2 env
    pub split_thr_m: FixpDbl,
    /// `split_thr_e`: Scale for splitting threshold
    pub split_thr_e: i32,
    /// `prevLowBandEnergy`: Energy of low band
    pub prev_low_band_energy: FixpDbl,
    /// `prevHighBandEnergy`: Energy of high band
    pub prev_high_band_energy: FixpDbl,
    /// `tran_fc`: Number of lowband subbands to discard
    pub tran_fc: i32,
    /// `no_cols`
    pub no_cols: i32,
    /// `no_rows`
    pub no_rows: i32,
    /// `mode`
    pub mode: i32,
    /// `frameShift`
    pub frame_shift: i32,
    /// `tran_off`: Offset for reading energy values.
    pub tran_off: i32,
}

impl Default for SbrTransientDetector {
    fn default() -> Self {
        SbrTransientDetector {
            transients: [0; 48],
            thresholds: [0; 64],
            tran_thr: 0,
            split_thr_m: 0,
            split_thr_e: 0,
            prev_low_band_energy: 0,
            prev_high_band_energy: 0,
            tran_fc: 0,
            no_cols: 0,
            no_rows: 0,
            mode: 0,
            frame_shift: 0,
            tran_off: 0,
        }
    }
}

/// `TRAN_DET_LOOKAHEAD`
pub const TRAN_DET_LOOKAHEAD: usize = 2;
/// `TRAN_DET_START_FREQ`: start frequency for transient detection
pub const TRAN_DET_START_FREQ: i32 = 4500;
/// `TRAN_DET_STOP_FREQ`: stop frequency for transient detection
pub const TRAN_DET_STOP_FREQ: i32 = 13500;
/// `TRAN_DET_MIN_QMFBANDS`: minimum qmf bands for transient detection
pub const TRAN_DET_MIN_QMFBANDS: i32 = 4;

/// `QMF_HP_dBd_SLOPE_FIX`: 0.002266f/10 * log2(10)
pub const QMF_HP_DBD_SLOPE_FIX: FixpDbl = fl2fxconst_dbl(0.00075275f32 as f64);
/// `TRAN_DET_THRSHLD`
pub const TRAN_DET_THRSHLD: FixpDbl = fl2fxconst_dbl((5.0f32 / 8.0f32) as f64);
/// `TRAN_DET_THRSHLD_SCALE`
pub const TRAN_DET_THRSHLD_SCALE: i32 = 3;

/// `FAST_TRAN_DETECTOR`
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FastTranDetector {
    /// `transientCandidates`
    pub transient_candidates: [i32; 32 + TRAN_DET_LOOKAHEAD],
    /// `nTimeSlots`
    pub n_time_slots: i32,
    /// `lookahead`
    pub lookahead: i32,
    /// `startBand`
    pub start_band: i32,
    /// `stopBand`
    pub stop_band: i32,

    /// `dBf_m`
    pub d_bf_m: [FixpDbl; 64],
    /// `dBf_e`
    pub d_bf_e: [i32; 64],

    /// `energy_timeSlots`
    pub energy_time_slots: [FixpDbl; 32 + TRAN_DET_LOOKAHEAD],
    /// `energy_timeSlots_scale`
    pub energy_time_slots_scale: [i32; 32 + TRAN_DET_LOOKAHEAD],

    /// `delta_energy`
    pub delta_energy: [FixpDbl; 32 + TRAN_DET_LOOKAHEAD],
    /// `delta_energy_scale`
    pub delta_energy_scale: [i32; 32 + TRAN_DET_LOOKAHEAD],

    /// `lowpass_energy`
    pub lowpass_energy: [FixpDbl; 32 + TRAN_DET_LOOKAHEAD],
    /// `lowpass_energy_scale`
    pub lowpass_energy_scale: [i32; 32 + TRAN_DET_LOOKAHEAD],
}

impl Default for FastTranDetector {
    fn default() -> Self {
        FastTranDetector {
            transient_candidates: [0; 34],
            n_time_slots: 0,
            lookahead: 0,
            start_band: 0,
            stop_band: 0,
            d_bf_m: [0; 64],
            d_bf_e: [0; 64],
            energy_time_slots: [0; 34],
            energy_time_slots_scale: [0; 34],
            delta_energy: [0; 34],
            delta_energy_scale: [0; 34],
            lowpass_energy: [0; 34],
            lowpass_energy_scale: [0; 34],
        }
    }
}

/// `ABS_THRES`: minimum threshold for detecting changes
const ABS_THRES: FixpDbl = 16;

/// `NRG_SHIFT`: for energy summation
const NRG_SHIFT: i32 = 3;

/// `LN2`: ln(2)
const LN2: FixpDbl = fl2fxconst_dbl(0.6931471806f32 as f64);

/// The combined energies of `FDKsbrEnc_frameSplitter()` (`EnergiesM`).
type EnergiesM = [[FixpDbl; MAX_FREQ_COEFFS]; NUMBER_TIME_SLOTS_2304 as usize];

/// `spectralChange(Energies, scaleEnergies, EnergyTotal, nSfb, start, border,
/// YBufferWriteOffset, stop, result_e)`: a measure for the spectral change within the frame
/// (how good it would be to split the frame at `border` into 2 envelopes), scaled by 1/64.
#[allow(clippy::too_many_arguments)]
fn spectral_change(
    energies: &EnergiesM,
    scale_energies: &[i32],
    mut energy_total: FixpDbl,
    n_sfb: i32,
    start: i32,
    border: i32,
    y_buffer_write_offset: i32,
    stop: i32,
    result_e: &mut i32,
) -> FixpDbl {
    let mut energies_e_diff = [0i8; NUMBER_TIME_SLOTS_2304 as usize];
    let mut energies_e: i8;
    let mut energy_total_e: i8 = 19;
    let energies_e_add: i8;
    let mut delta_sum: FixpDbl = 0;
    let mut tmp_e = 0;

    *result_e = 0;

    let len1 = border - start;
    let len2 = stop - border;

    // prefer borders near the middle of the frame
    let mut pos_weight =
        fl2fxconst_dbl(0.5f32 as f64).wrapping_sub(len1.wrapping_mul(get_inv_int(len1 + len2)));
    pos_weight = MAXVAL_DBL.wrapping_sub(f_mult(pos_weight, pos_weight) << 2);

    // Calc scaling for energies
    fdk_assert!(scale_energies[0] >= 0);
    fdk_assert!(scale_energies[1] >= 0);

    energies_e = (19 - f_min(scale_energies[0], scale_energies[1])) as i8;

    // limit shift for energy accumulation, energies_e can be -10 min.
    if energies_e < -10 {
        energies_e_add = -10 - energies_e;
        energies_e = -10;
    } else if energies_e > 17 {
        energies_e_add = energies_e - 17;
        energies_e = 17;
    } else {
        energies_e_add = 0;
    }

    // compensate scaling differences between scaleEnergies[0] and scaleEnergies[1]
    let mut prev_energies_e_diff = (scale_energies[0] - f_min(scale_energies[0], scale_energies[1])
        + energies_e_add as i32
        + NRG_SHIFT) as i8;
    let mut new_energies_e_diff = (scale_energies[1] - f_min(scale_energies[0], scale_energies[1])
        + energies_e_add as i32
        + NRG_SHIFT) as i8;

    prev_energies_e_diff = f_min(prev_energies_e_diff as i32, DFRACT_BITS - 1) as i8;
    new_energies_e_diff = f_min(new_energies_e_diff as i32, DFRACT_BITS - 1) as i8;

    for i in start..y_buffer_write_offset {
        energies_e_diff[i as usize] = prev_energies_e_diff;
    }
    for i in y_buffer_write_offset..stop {
        energies_e_diff[i as usize] = new_energies_e_diff;
    }

    // Sum up energies of all QMF-timeslots for both halfs
    fdk_assert!(len1 <= 8); // otherwise an overflow is possible
    fdk_assert!(len2 <= 8); // otherwise an overflow is possible

    // fLog2(len1, 31) - fLog2(len2, 31) of every band, once (a pure function of loop-invariant
    // values)
    let tmp1 = f_log2_noexp(len1, 31).wrapping_sub(f_log2_noexp(len2, 31));
    for j in 0..n_sfb.max(0) as usize {
        let mut accu1: FixpDbl = 0;
        let mut accu2: FixpDbl = 0;
        let mut accu_e = energies_e as i32 + 3;

        // Sum up energies in first half
        for i in start..border {
            let i = i as usize;
            accu1 = accu1.wrapping_add(scale_value(energies[i][j], -(energies_e_diff[i] as i32)));
        }

        // Sum up energies in second half
        for i in border..stop {
            let i = i as usize;
            accu2 = accu2.wrapping_add(scale_value(energies[i][j], -(energies_e_diff[i] as i32)));
        }

        // Ensure certain energy to prevent division by zero and to prevent splitting for very
        // low levels
        accu1 = f_max(accu1, len1);
        accu2 = f_max(accu2, len2);

        // Energy change in current band
        let tmp0 = f_log2_noexp(accu2, accu_e).wrapping_sub(f_log2_noexp(accu1, accu_e));
        let mut delta = f_mult(LN2, tmp0.wrapping_add(tmp1));
        delta = f_abs(delta);

        // Weighting with amplitude ratio of this band
        accu_e += 1; // scale at least one bit due to (accu1+accu2)
        accu1 >>= 1;
        accu2 >>= 1;

        if accu_e & 1 != 0 {
            accu_e += 1; // for a defined square result exponent, the exponent has to be even
            accu1 >>= 1;
            accu2 >>= 1;
        }

        delta_sum = delta_sum.wrapping_add(f_mult(sqrt_fixp(accu1.wrapping_add(accu2)), delta));
        *result_e = (accu_e >> 1) + LD_DATA_SHIFT;
    }

    if energy_total_e & 1 != 0 {
        energy_total_e += 1; // for a defined square result exponent, the exponent has to be even
        energy_total >>= 1;
    }

    delta_sum = f_mult(delta_sum, inv_sqrt_norm2(energy_total, &mut tmp_e));
    *result_e += tmp_e - (energy_total_e as i32 >> 1);

    f_mult(delta_sum, pos_weight)
}

/// `addLowbandEnergies(Energies, scaleEnergies, YBufferWriteOffset, nrgSzShift, tran_off,
/// freqBandTable, slots)`: the total lowband energy, scaled by 2^19.
fn add_lowband_energies(
    energies: &[&[FixpDbl]],
    scale_energies: &[i32],
    y_buffer_write_offset: i32,
    nrg_sz_shift: i32,
    tran_off: i32,
    freq_band_table: &[u8],
    slots: i32,
) -> FixpDbl {
    let mut nrg_total_e = 0;
    let mut accu1: FixpDbl = 0;
    let mut accu2: FixpDbl = 0;
    let tran_offdiv2 = tran_off >> nrg_sz_shift;
    let fbt0 = freq_band_table[0] as i32;
    let sc1 = DFRACT_BITS
        - f_normz(f_max(1, fbt0.wrapping_mul(y_buffer_write_offset - tran_offdiv2) - 1));
    let sc2 = DFRACT_BITS
        - f_normz(f_max(
            1,
            fbt0.wrapping_mul(tran_offdiv2 + (slots >> nrg_sz_shift) - y_buffer_write_offset) - 1,
        ));

    // Sum up lowband energy from one frame at offset tran_off
    // freqBandTable[LORES] has MAX_FREQ_COEFFS/2 +1 coeefs max.
    let mut ts = tran_offdiv2;
    while ts < y_buffer_write_offset {
        for k in 0..fbt0 as usize {
            accu1 = accu1.wrapping_add(energies[ts as usize][k] >> sc1);
        }
        ts += 1;
    }
    while ts < tran_offdiv2 + (slots >> nrg_sz_shift) {
        for k in 0..fbt0 as usize {
            accu2 = accu2.wrapping_add(energies[ts as usize][k] >> sc2);
        }
        ts += 1;
    }

    let nrg_total_m = f_add_norm(
        accu1,
        (sc1 - 5) - scale_energies[0],
        accu2,
        (sc2 - 5) - scale_energies[1],
        &mut nrg_total_e,
    );
    scale_value_saturate(nrg_total_m, nrg_total_e)
}

/// `addHighbandEnergies(Energies, scaleEnergies, YBufferWriteOffset, EnergiesM, freqBandTable,
/// nSfb, sbrSlots, timeStep)`: maps the highband energies to SBR time slots and SBR bands
/// (`energies_m`) and returns the total energy in the highband, scaled by 2^19.
#[allow(clippy::too_many_arguments)]
fn add_highband_energies(
    energies: &[&[FixpDbl]],
    scale_energies: &[i32],
    y_buffer_write_offset: i32,
    energies_m: &mut EnergiesM,
    freq_band_table: &[u8],
    n_sfb: i32,
    sbr_slots: i32,
    time_step: i32,
) -> FixpDbl {
    let nrg_total;

    // Combine QMF-timeslots to SBR-timeslots, combine QMF-bands to SBR-bands, combine Left and
    // Right channel
    for slot_out in 0..sbr_slots.max(0) as usize {
        // Note: Below slotIn = slotOut and not slotIn = timeStep*slotOut because the
        // Energies[] time resolution is always the SBR slot resolution regardless of the
        // timeStep.
        let slot_in = slot_out;

        for j in 0..n_sfb.max(0) as usize {
            let mut accu: FixpDbl = 0;

            let li = freq_band_table[j] as usize;
            let ui = freq_band_table[j + 1] as usize;

            for k in li..ui {
                for _ in 0..time_step {
                    accu = accu.wrapping_add(energies[slot_in][k] >> 5);
                }
            }
            energies_m[slot_out][j] = accu;
        }
    }

    // scale energies down before add up
    let scale = [fix_min(8, scale_energies[0]), fix_min(8, scale_energies[1])];

    if (scale_energies[0] - scale[0]) > (DFRACT_BITS - 1)
        || (scale_energies[1] - scale[1]) > (DFRACT_BITS - 1)
    {
        nrg_total = 0;
    } else {
        // Now add all energies
        let mut accu: FixpDbl = 0;

        for slot_out in 0..y_buffer_write_offset.max(0) as usize {
            for j in 0..n_sfb.max(0) as usize {
                accu = accu.wrapping_add(energies_m[slot_out][j] >> scale[0]);
            }
        }
        let partial = accu >> (scale_energies[0] - scale[0]);

        for slot_out in y_buffer_write_offset.max(0) as usize..sbr_slots.max(0) as usize {
            for j in 0..n_sfb.max(0) as usize {
                accu = accu.wrapping_add(energies_m[slot_out][j] >> scale[0]);
            }
        }
        nrg_total = f_add_saturate(partial, accu >> (scale_energies[1] - scale[1]));
    }

    nrg_total
}

/// `FDKsbrEnc_frameSplitter(Energies, scaleEnergies, h_sbrTransientDetector, freqBandTable,
/// tran_vector, YBufferWriteOffset, YBufferSzShift, nSfb, timeStep, no_cols, tonality)`:
/// decides if a FIXFIX frame shall be split into 2 envelopes (if no transient has been
/// detected).
#[allow(clippy::too_many_arguments)]
pub fn fdk_sbr_enc_frame_splitter(
    energies: &[&[FixpDbl]],
    scale_energies: &[i32],
    h_sbr_transient_detector: &mut SbrTransientDetector,
    freq_band_table: &[u8],
    tran_vector: &mut [u8],
    y_buffer_write_offset: i32,
    y_buffer_sz_shift: i32,
    n_sfb: i32,
    time_step: i32,
    no_cols: i32,
    tonality: &mut FixpDbl,
) {
    if tran_vector[1] == 0 {
        // no transient was detected
        let delta;
        let mut delta_e = 0;
        let sbr_slots = f_mult_i(get_inv_int(time_step), no_cols);
        let mut energies_m: EnergiesM = [[0; MAX_FREQ_COEFFS]; NUMBER_TIME_SLOTS_2304 as usize];

        fdk_assert!(sbr_slots * time_step == no_cols);

        // Get Lowband-energy over a range of 2 frames (Look half a frame back and ahead).
        let new_lowband_energy = add_lowband_energies(
            energies,
            scale_energies,
            y_buffer_write_offset,
            y_buffer_sz_shift,
            h_sbr_transient_detector.tran_off,
            freq_band_table,
            no_cols,
        );

        let new_highband_energy = add_highband_energies(
            energies,
            scale_energies,
            y_buffer_write_offset,
            &mut energies_m,
            freq_band_table,
            n_sfb,
            sbr_slots,
            time_step,
        );

        {
            // prevLowBandEnergy: Corresponds to 1 frame, starting with half a frame
            // look-behind; newLowbandEnergy: Corresponds to 1 frame, starting in the middle of
            // the current frame
            let mut energy_total = (new_lowband_energy >> 1)
                .wrapping_add(h_sbr_transient_detector.prev_low_band_energy >> 1); // mean of new and prev LB NRG
            energy_total = f_add_saturate(energy_total, new_highband_energy); // Add HB NRG
                                                                              // The below border should specify the same position as the middle border of a
                                                                              // FIXFIX-frame with 2 envelopes.
            let border = (sbr_slots + 1) >> 1;

            if (energy_total as u32 & 0xffff_ffe0) != 0
                && (scale_energies[0] < 32 || scale_energies[1] < 32)
            {
                // i.e. > 31
                delta = spectral_change(
                    &energies_m,
                    scale_energies,
                    energy_total,
                    n_sfb,
                    0,
                    border,
                    y_buffer_write_offset,
                    sbr_slots,
                    &mut delta_e,
                );
            } else {
                delta = 0;
                delta_e = 0;

                // set tonality to 0 when energy is very low, since the amplitude resolution
                // should then be low as well
                *tonality = 0;
            }

            if f_is_less_than(
                h_sbr_transient_detector.split_thr_m,
                h_sbr_transient_detector.split_thr_e,
                delta,
                delta_e,
            ) != 0
            {
                tran_vector[0] = 1; // Set flag for splitting
            } else {
                tran_vector[0] = 0;
            }
        }

        // Update prevLowBandEnergy
        h_sbr_transient_detector.prev_low_band_energy = new_lowband_energy;
        h_sbr_transient_detector.prev_high_band_energy = new_highband_energy;
    }
}

/// `calculateThresholds(Energies, scaleEnergies, thresholds, YBufferWriteOffset,
/// YBufferSzShift, noCols, noRows, tran_off)`: the transient energy threshold of each QMF band.
#[allow(clippy::too_many_arguments)]
#[doc(hidden)]
pub fn calculate_thresholds_straight(
    energies: &[&[FixpDbl]],
    scale_energies: &[i32],
    thresholds: &mut [FixpDbl],
    y_buffer_write_offset: i32,
    y_buffer_sz_shift: i32,
    no_cols: i32,
    no_rows: i32,
    tran_off: i32,
) {
    let i_no_cols = get_inv_int(no_cols + tran_off) << y_buffer_sz_shift;
    let i_no_cols1 = get_inv_int(no_cols + tran_off - 1) << y_buffer_sz_shift;

    // calc minimum scale of energies of previous and current frame
    let common_scale = fix_min(scale_energies[0], scale_energies[1]);

    // calc scalefactors to adapt energies to common scale
    let scale_factor0 = fix_min(scale_energies[0] - common_scale, DFRACT_BITS - 1);
    let scale_factor1 = fix_min(scale_energies[1] - common_scale, DFRACT_BITS - 1);

    fdk_assert!(scale_factor0 >= 0 && scale_factor1 >= 0);

    // calculate standard deviation in every subband
    for i in 0..no_rows.max(0) as usize {
        let start_energy = tran_off >> y_buffer_sz_shift;
        let end_energy = (no_cols >> y_buffer_sz_shift) + tran_off;

        // calculate mean value over decimated energy values (downsampled by 2).
        let mut accu0: FixpDbl = 0;
        let mut accu1: FixpDbl = 0;

        let mut j = start_energy;
        while j < y_buffer_write_offset {
            accu0 = f_mult_add_div2(accu0, energies[j as usize][i], i_no_cols);
            j += 1;
        }
        while j < end_energy {
            accu1 = f_mult_add_div2(accu1, energies[j as usize][i], i_no_cols);
            j += 1;
        }

        let mean_val = ((accu0 << 1) >> scale_factor0).wrapping_add((accu1 << 1) >> scale_factor1); // average
        let shift = fix_max(0, count_leading_bits(mean_val) - 6); // -6 to keep room for accumulating upto N = 24 values

        // calculate standard deviation
        let mut accu: FixpDbl = 0;

        // summe { ((mean_val-nrg)^2) * i_noCols1 }
        let mut j = start_energy;
        while j < y_buffer_write_offset {
            let mut temp = mean_val
                .wrapping_sub(energies[j as usize][i] >> scale_factor0)
                .wrapping_shl(shift as u32);
            temp = f_pow2_div2(temp);
            accu = f_mult_add_div2(accu, temp, i_no_cols1);
            j += 1;
        }
        while j < end_energy {
            let mut temp = mean_val
                .wrapping_sub(energies[j as usize][i] >> scale_factor1)
                .wrapping_shl(shift as u32);
            temp = f_pow2_div2(temp);
            accu = f_mult_add_div2(accu, temp, i_no_cols1);
            j += 1;
        }
        accu <<= 2;
        let std_val = sqrt_fixp(accu) >> shift; // standard deviation

        // Take new threshold as average of calculated standard deviation ratio and old
        // threshold if greater than absolute threshold
        let temp = if common_scale <= DFRACT_BITS - 1 {
            f_mult(fl2fxconst_dbl(0.66f32 as f64), thresholds[i])
                .wrapping_add(f_mult(fl2fxconst_dbl(0.34f32 as f64), std_val) >> common_scale)
        } else {
            0
        };

        thresholds[i] = fix_max(ABS_THRES, temp);

        fdk_assert!(common_scale >= 0);
    }
}

/// The bands of the energy rows (`noRows`, at most 64 QMF bands).
const MAX_ROWS: usize = 64;

/// [`calculate_thresholds_straight`] with the loops over the time slots outside and the loops
/// over the bands inside (each band accumulates the same terms in the same order; the bands are
/// independent), so that the rows are read contiguously and the band loops vectorize.
#[allow(clippy::too_many_arguments)]
#[doc(hidden)]
pub fn calculate_thresholds(
    energies: &[&[FixpDbl]],
    scale_energies: &[i32],
    thresholds: &mut [FixpDbl],
    y_buffer_write_offset: i32,
    y_buffer_sz_shift: i32,
    no_cols: i32,
    no_rows: i32,
    tran_off: i32,
) {
    if no_rows as u32 > MAX_ROWS as u32 || tran_off >> y_buffer_sz_shift < 0 {
        return calculate_thresholds_straight(
            energies,
            scale_energies,
            thresholds,
            y_buffer_write_offset,
            y_buffer_sz_shift,
            no_cols,
            no_rows,
            tran_off,
        );
    }
    let i_no_cols = get_inv_int(no_cols + tran_off) << y_buffer_sz_shift;
    let i_no_cols1 = get_inv_int(no_cols + tran_off - 1) << y_buffer_sz_shift;

    // calc minimum scale of energies of previous and current frame
    let common_scale = fix_min(scale_energies[0], scale_energies[1]);

    // calc scalefactors to adapt energies to common scale
    let scale_factor0 = fix_min(scale_energies[0] - common_scale, DFRACT_BITS - 1);
    let scale_factor1 = fix_min(scale_energies[1] - common_scale, DFRACT_BITS - 1);

    fdk_assert!(scale_factor0 >= 0 && scale_factor1 >= 0);

    let n = no_rows.max(0) as usize;
    let start_energy = tran_off >> y_buffer_sz_shift;
    let end_energy = (no_cols >> y_buffer_sz_shift) + tran_off;
    // the first loop runs from startEnergy to YBufferWriteOffset, the second from there (or from
    // startEnergy) to endEnergy
    let mid = start_energy.max(y_buffer_write_offset);
    let range = |a: i32, b: i32| (a as usize)..(b.max(a) as usize);
    let (first, second) = (range(start_energy, mid), range(mid, end_energy));

    // calculate mean value over decimated energy values (downsampled by 2).
    let mut accu0 = [0 as FixpDbl; MAX_ROWS];
    let mut accu1 = [0 as FixpDbl; MAX_ROWS];
    for j in first.clone() {
        let row = &energies[j][..n];
        for (a, &e) in accu0[..n].iter_mut().zip(row) {
            *a = f_mult_add_div2(*a, e, i_no_cols);
        }
    }
    for j in second.clone() {
        let row = &energies[j][..n];
        for (a, &e) in accu1[..n].iter_mut().zip(row) {
            *a = f_mult_add_div2(*a, e, i_no_cols);
        }
    }
    let mut mean_val = [0 as FixpDbl; MAX_ROWS];
    let mut shift = [0i32; MAX_ROWS];
    for i in 0..n {
        mean_val[i] =
            ((accu0[i] << 1) >> scale_factor0).wrapping_add((accu1[i] << 1) >> scale_factor1); // average
        shift[i] = fix_max(0, count_leading_bits(mean_val[i]) - 6); // -6 to keep room for accumulating upto N = 24 values
    }

    // calculate standard deviation: summe { ((mean_val-nrg)^2) * i_noCols1 }
    let mut accu = [0 as FixpDbl; MAX_ROWS];
    for (js, sf) in [(first, scale_factor0), (second, scale_factor1)] {
        for j in js {
            let row = &energies[j][..n];
            for i in 0..n {
                let mut temp = mean_val[i].wrapping_sub(row[i] >> sf).wrapping_shl(shift[i] as u32);
                temp = f_pow2_div2(temp);
                accu[i] = f_mult_add_div2(accu[i], temp, i_no_cols1);
            }
        }
    }

    for i in 0..n {
        let std_val = sqrt_fixp(accu[i] << 2) >> shift[i]; // standard deviation

        // Take new threshold as average of calculated standard deviation ratio and old
        // threshold if greater than absolute threshold
        let temp = if common_scale <= DFRACT_BITS - 1 {
            f_mult(fl2fxconst_dbl(0.66f32 as f64), thresholds[i])
                .wrapping_add(f_mult(fl2fxconst_dbl(0.34f32 as f64), std_val) >> common_scale)
        } else {
            0
        };

        thresholds[i] = fix_max(ABS_THRES, temp);

        fdk_assert!(common_scale >= 0);
    }
}

/// The peak detection of [`extract_transient_candidates`] for one band: adds the transient
/// level of the column with the energies around `e[j0 + c]` to `transients[c]` for every
/// column `c`. The straight loop per column,
///
/// ```text
/// for d in 1..4 { delta += e[j + d] - e[j - d] - thres; if delta > 0 { tran = fMultAddDiv2(tran, iThres, delta) } }
/// ```
///
/// as the three partial sums `delta_d` and the selected products `fMultDiv2(iThres, delta_d)`
/// (`fMultAddDiv2(x, a, b) = x + fMultDiv2(a, b)` on every variant), summed with wrapping
/// additions in any order, without branches. The columns are independent, so the loop
/// vectorizes (on x86 compiled for AVX2 if available, for the 64-bit products).
#[inline]
fn transient_columns(
    e: &[FixpDbl],
    j0: usize,
    thres: FixpDbl,
    i_thres: FixpDbl,
    out: &mut [FixpDbl],
) {
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    if crate::simd::avx2() {
        #[target_feature(enable = "avx2")]
        unsafe fn avx2(
            e: &[FixpDbl],
            j0: usize,
            thres: FixpDbl,
            i_thres: FixpDbl,
            out: &mut [FixpDbl],
        ) {
            transient_columns_impl(e, j0, thres, i_thres, out)
        }
        // SAFETY: the CPU supports AVX2.
        return unsafe { avx2(e, j0, thres, i_thres, out) };
    }
    transient_columns_impl(e, j0, thres, i_thres, out)
}

#[inline(always)]
fn transient_columns_impl(
    e: &[FixpDbl],
    j0: usize,
    thres: FixpDbl,
    i_thres: FixpDbl,
    out: &mut [FixpDbl],
) {
    let n = out.len();
    let at = |d: isize| &e[(j0 as isize + d) as usize..(j0 as isize + d) as usize + n];
    let (l3, l2, l1, r1, r2, r3) = (at(-3), at(-2), at(-1), at(1), at(2), at(3));
    let term = |delta: FixpDbl| -> FixpDbl {
        if delta > 0 {
            f_mult_div2(i_thres, delta)
        } else {
            0
        }
    };
    for c in 0..n {
        let d1 = r1[c].wrapping_sub(l1[c]).wrapping_sub(thres);
        let d2 = d1.wrapping_add(r2[c]).wrapping_sub(l2[c]).wrapping_sub(thres);
        let d3 = d2.wrapping_add(r3[c]).wrapping_sub(l3[c]).wrapping_sub(thres);
        let tran = term(d1).wrapping_add(term(d2)).wrapping_add(term(d3));
        out[c] = out[c].wrapping_add(tran << 1);
    }
}

/// `extractTransientCandidates(Energies, scaleEnergies, thresholds, transients,
/// YBufferWriteOffset, YBufferSzShift, noCols, start_band, stop_band, tran_off,
/// addPrevSamples)`: the transient levels of each QMF time slot.
#[allow(clippy::too_many_arguments)]
fn extract_transient_candidates(
    energies: &[&[FixpDbl]],
    scale_energies: &[i32],
    thresholds: &[FixpDbl],
    transients: &mut [FixpDbl],
    y_buffer_write_offset: i32,
    y_buffer_sz_shift: i32,
    no_cols: i32,
    start_band: i32,
    stop_band: i32,
    tran_off: i32,
    add_prev_samples: i32,
) {
    let mut energies_temp = [0 as FixpDbl; 2 * 32];

    // Scale value for first energies, upto YBufferWriteOffset
    let tmp_scale_energies0 = fix_min(scale_energies[0], MAX_SHIFT_DBL);
    // Scale value for first energies, from YBufferWriteOffset upwards
    let tmp_scale_energies1 = fix_min(scale_energies[1], MAX_SHIFT_DBL);

    fdk_assert!(tmp_scale_energies0 >= 0 && tmp_scale_energies1 >= 0);

    // Keep addPrevSamples extra previous transient candidates.
    let src = (no_cols - add_prev_samples) as usize;
    let n = (tran_off + add_prev_samples) as usize;
    transients.copy_within(src..src + n, 0);
    let c0 = (tran_off + add_prev_samples) as usize;
    transients[c0..c0 + no_cols as usize].fill(0);

    let end_cond = no_cols; // Amount of new transient values to be calculated.
    let start_energ = (tran_off - 3) >> y_buffer_sz_shift; // >>YBufferSzShift because of amount of energy values. -3 because of neighbors being watched.
    let end_energ =
        ((no_cols + (y_buffer_write_offset << y_buffer_sz_shift)) - 1) >> y_buffer_sz_shift; // YBufferSzShift shifts because of half energy values.

    // Compute differential values with two different weightings in every subband
    for i in start_band.max(0) as usize..stop_band.max(0) as usize {
        let thres = thresholds[i];

        let i_thres = if thresholds[i] >= 256 {
            (MAXVAL_DBL / (thresholds[i] + 1)) << (32 - 24)
        } else {
            MAXVAL_DBL
        };

        // Copy one timeslot and de-scale and de-squish
        if y_buffer_sz_shift == 1 {
            let mut j = start_energ;
            while j < y_buffer_write_offset {
                let tmp = energies[j as usize][i];
                let v = tmp >> tmp_scale_energies0;
                energies_temp[((j << 1) + 1) as usize] = v;
                energies_temp[(j << 1) as usize] = v;
                j += 1;
            }
            while j <= end_energ {
                let tmp = energies[j as usize][i];
                let v = tmp >> tmp_scale_energies1;
                energies_temp[((j << 1) + 1) as usize] = v;
                energies_temp[(j << 1) as usize] = v;
                j += 1;
            }
        } else {
            let mut j = start_energ;
            while j < y_buffer_write_offset {
                let tmp = energies[j as usize][i];
                energies_temp[j as usize] = tmp >> tmp_scale_energies0;
                j += 1;
            }
            while j <= end_energ {
                let tmp = energies[j as usize][i];
                energies_temp[j as usize] = tmp >> tmp_scale_energies1;
                j += 1;
            }
        }

        // Detect peaks in energy values.

        let mut j_index = tran_off as usize;
        let mut jp_bm = j_index + add_prev_samples as usize;
        let cols = end_cond.max(0) as usize;
        if j_index >= 3
            && j_index + cols + 3 <= energies_temp.len()
            && jp_bm + cols <= transients.len()
        {
            transient_columns(
                &energies_temp,
                j_index,
                thres,
                i_thres,
                &mut transients[jp_bm..jp_bm + cols],
            );
            continue;
        }

        for _ in 0..end_cond.max(0) {
            let mut delta: FixpDbl = 0;
            let mut tran: FixpDbl = 0;

            for d in 1..4usize {
                delta = delta.wrapping_add(energies_temp[j_index + d]); // R
                delta = delta.wrapping_sub(energies_temp[j_index - d]); // L
                delta = delta.wrapping_sub(thres);

                if delta > 0 {
                    tran = f_mult_add_div2(tran, i_thres, delta);
                }
            }
            transients[jp_bm] = transients[jp_bm].wrapping_add(tran << 1);
            j_index += 1;
            jp_bm += 1;
        }
    }
}

/// `FDKsbrEnc_transientDetect(h_sbrTran, Energies, scaleEnergies, transient_info,
/// YBufferWriteOffset, YBufferSzShift, timeStep, frameMiddleBorder)`
#[allow(clippy::too_many_arguments)]
pub fn fdk_sbr_enc_transient_detect(
    h_sbr_tran: &mut SbrTransientDetector,
    energies: &[&[FixpDbl]],
    scale_energies: &[i32],
    transient_info: &mut [u8],
    y_buffer_write_offset: i32,
    y_buffer_sz_shift: i32,
    time_step: i32,
    frame_middle_border: i32,
) {
    let no_cols = h_sbr_tran.no_cols;

    // Where to start looking for transients in the transient candidate buffer
    let mut qmf_start_sample = time_step * frame_middle_border;
    // We need to look one value backwards in the transients, so we might need one more previous
    // value.
    let add_prev_samples = if qmf_start_sample > 0 { 0 } else { 1 };

    let time_step_shift = match time_step {
        1 => 0,
        2 => 1,
        4 => 2,
        _ => 0,
    };

    calculate_thresholds(
        energies,
        scale_energies,
        &mut h_sbr_tran.thresholds,
        y_buffer_write_offset,
        y_buffer_sz_shift,
        h_sbr_tran.no_cols,
        h_sbr_tran.no_rows,
        h_sbr_tran.tran_off,
    );

    extract_transient_candidates(
        energies,
        scale_energies,
        &h_sbr_tran.thresholds,
        &mut h_sbr_tran.transients,
        y_buffer_write_offset,
        y_buffer_sz_shift,
        h_sbr_tran.no_cols,
        0,
        h_sbr_tran.no_rows,
        h_sbr_tran.tran_off,
        add_prev_samples,
    );

    transient_info[0] = 0;
    transient_info[1] = 0;
    transient_info[2] = 0;

    // Offset by the amount of additional previous transient candidates being kept.
    qmf_start_sample += add_prev_samples;

    // Check for transients in second granule (pick the last value of subsequent values)
    for i in qmf_start_sample..qmf_start_sample + no_cols {
        let iu = i as usize;
        let cond = (h_sbr_tran.transients[iu]
            < f_mult(fl2fxconst_dbl(0.9f32 as f64), h_sbr_tran.transients[iu - 1]))
            && (h_sbr_tran.transients[iu - 1] > h_sbr_tran.tran_thr);

        if cond {
            transient_info[0] = ((i - qmf_start_sample) >> time_step_shift) as u8;
            transient_info[1] = 1;
            break;
        }
    }

    if h_sbr_tran.frame_shift != 0 {
        // transient prediction for LDSBR
        // Check for transients in first <frameShift> qmf-slots of second frame
        for i in qmf_start_sample + no_cols..qmf_start_sample + no_cols + h_sbr_tran.frame_shift {
            let iu = i as usize;
            let cond = (h_sbr_tran.transients[iu]
                < f_mult(fl2fxconst_dbl(0.9f32 as f64), h_sbr_tran.transients[iu - 1]))
                && (h_sbr_tran.transients[iu - 1] > h_sbr_tran.tran_thr);

            if cond {
                let pos = (i - qmf_start_sample - no_cols) >> time_step_shift;
                if pos < 3 && transient_info[1] == 0 {
                    transient_info[2] = 1;
                }
                break;
            }
        }
    }
}

/// `FDKsbrEnc_InitSbrTransientDetector(h_sbrTransientDetector, sbrSyntaxFlags, frameSize,
/// sampleFreq, params, tran_fc, no_cols, no_rows, YBufferWriteOffset, YBufferSzShift,
/// frameShift, tran_off)`: returns 0.
#[allow(clippy::too_many_arguments)]
pub fn fdk_sbr_enc_init_sbr_transient_detector(
    h_sbr_transient_detector: &mut SbrTransientDetector,
    sbr_syntax_flags: u32,
    frame_size: i32,
    sample_freq: i32,
    params: &SbrConfiguration,
    tran_fc: i32,
    no_cols: i32,
    no_rows: i32,
    _y_buffer_write_offset: i32,
    _y_buffer_sz_shift: i32,
    frame_shift: i32,
    tran_off: i32,
) -> i32 {
    let total_bitrate =
        params.codec_settings.standard_bitrate.wrapping_mul(params.codec_settings.n_channels);
    let codec_bitrate = params.codec_settings.bit_rate;
    let bitrate_factor_m;
    let mut bitrate_factor_e = 0;
    let mut tmp_e = 0;

    *h_sbr_transient_detector = SbrTransientDetector::default();

    h_sbr_transient_detector.frame_shift = frame_shift;
    h_sbr_transient_detector.tran_off = tran_off;

    if codec_bitrate != 0 {
        bitrate_factor_m =
            f_div_norm(total_bitrate, codec_bitrate.wrapping_shl(2), &mut bitrate_factor_e);
        bitrate_factor_e += 2;
    } else {
        bitrate_factor_m = fl2fxconst_dbl(1.0 / 4.0);
        bitrate_factor_e = 2;
    }

    let framedur_fix = f_div_norm_noexp(frame_size, sample_freq);

    // The longer the frames, the more often should the FIXFIX-case transmit 2 envelopes
    // instead of 1. Frame durations below 10 ms produce the highest threshold so that
    // practically always only 1 env is transmitted.
    let mut tmp = framedur_fix.wrapping_sub(fl2fxconst_dbl(0.010));

    tmp = fix_max(tmp, fl2fxconst_dbl(0.0001));
    tmp = f_div_norm(fl2fxconst_dbl(0.000075), f_pow2(tmp), &mut tmp_e);

    bitrate_factor_e += tmp_e;

    if sbr_syntax_flags & SBR_SYNTAX_LOW_DELAY != 0 {
        bitrate_factor_e -= 1; // divide by 2
    }

    fdk_assert!(no_cols <= 32);
    fdk_assert!(no_rows <= 64);

    h_sbr_transient_detector.no_cols = no_cols;
    h_sbr_transient_detector.tran_thr =
        if no_rows != 0 { params.tran_thr.wrapping_shl((32 - 24 - 1) as u32) / no_rows } else { 0 };
    h_sbr_transient_detector.tran_fc = tran_fc;
    h_sbr_transient_detector.split_thr_m = f_mult(tmp, bitrate_factor_m);
    h_sbr_transient_detector.split_thr_e = bitrate_factor_e;
    h_sbr_transient_detector.no_rows = no_rows;
    h_sbr_transient_detector.mode = params.tran_det_mode;
    h_sbr_transient_detector.prev_low_band_energy = 0;

    0
}

/// `ENERGY_SCALING_SIZE`
pub const ENERGY_SCALING_SIZE: i32 = 32;

/// `EXP_E`: 64 (=64) multiplications max, max. allowed sum is 0.5
const EXP_E: i32 = 7;

/// `FDKsbrEnc_InitSbrFastTransientDetector(h_sbrFastTransientDetector, time_slots_per_frame,
/// bandwidth_qmf_slot, no_qmf_channels, sbr_qmf_1st_band)`: returns 0.
pub fn fdk_sbr_enc_init_sbr_fast_transient_detector(
    h: &mut FastTranDetector,
    time_slots_per_frame: i32,
    bandwidth_qmf_slot: i32,
    no_qmf_channels: i32,
    sbr_qmf_1st_band: i32,
) -> i32 {
    h.lookahead = TRAN_DET_LOOKAHEAD as i32;
    h.n_time_slots = time_slots_per_frame;

    let buff_size = (h.n_time_slots + h.lookahead) as usize;

    for i in 0..buff_size.min(h.delta_energy.len()) {
        h.delta_energy[i] = 0;
        h.energy_time_slots[i] = 0;
        h.lowpass_energy[i] = 0;
        h.transient_candidates[i] = 0;
    }

    fdk_assert!(bandwidth_qmf_slot > 0);
    h.stop_band = if bandwidth_qmf_slot != 0 {
        f_min(TRAN_DET_STOP_FREQ / bandwidth_qmf_slot, no_qmf_channels)
    } else {
        no_qmf_channels
    };
    h.start_band = f_min(sbr_qmf_1st_band, h.stop_band - TRAN_DET_MIN_QMFBANDS);

    fdk_assert!(h.start_band < no_qmf_channels);
    fdk_assert!(h.start_band < h.stop_band);
    fdk_assert!(h.start_band > 1);
    fdk_assert!(h.stop_band > 1);

    // the energy weighting and adding up has a headroom of 6 Bits, so up to 64 bands can be
    // added without potential overflow.
    fdk_assert!(h.stop_band - h.start_band <= 64);

    // QMF_HP_dB_SLOPE_FIX says that we want a 20 dB per 16 kHz HP filter. The following lines
    // map this to the QMF bandwidth.
    let my_exp =
        f_mult_norm_to(QMF_HP_DBD_SLOPE_FIX, 0, bandwidth_qmf_slot, DFRACT_BITS - 1, EXP_E);
    let mut my_exp_slot = my_exp;

    for i in 0..64 {
        // Calculate dBf over all qmf bands:
        // dBf = (10^(0.002266f/10*bw(slot)))^(band) =
        //     = 2^(log2(10)*0.002266f/10*bw(slot)*band) =
        //     = 2^(0.00075275f*bw(slot)*band)

        let d_bf_m: FixpDbl; // dBf mantissa
        let d_bf_e: i32; // dBf exponent
        let mut tmp = 0;

        // myExp*(i+1) = myExp_int - myExp_fract
        // myExp*(i+1) is split up here for better accuracy of CalcInvLdData(), for its result
        // can be split up into an integer and a fractional part

        // Round up to next integer
        let my_exp_int = (my_exp_slot & 0xfe00_0000u32 as i32).wrapping_add(0x0200_0000);

        // This is the fractional part that needs to be substracted
        let my_exp_fract = my_exp_int.wrapping_sub(my_exp_slot);

        // Calc integer part
        let mut d_bf_int = calc_inv_ld_data(my_exp_int);
        // The result needs to be re-scaled. The ld(myExp_int) had been scaled by EXP_E, the
        // CalcInvLdData expects the operand to be scaled by LD_DATA_SHIFT. Therefore, the
        // correctly scaled result is dBf_int^(2^(EXP_E-LD_DATA_SHIFT)), which is dBf_int^2

        if d_bf_int <= 46340 {
            // compare with maximum allowed value for signed integer multiplication, 46340 =
            // (INT)floor(sqrt((double)(((UINT)1<<(DFRACT_BITS-1))-1)))
            d_bf_int *= d_bf_int;

            // Calc fractional part
            let mut d_bf_fract = calc_inv_ld_data(my_exp_fract.wrapping_neg());
            // The result needs to be re-scaled. The ld(myExp_fract) had been scaled by EXP_E,
            // the CalcInvLdData expects the operand to be scaled by LD_DATA_SHIFT. Therefore,
            // the correctly scaled result is dBf_fract^(2^(EXP_E-LD_DATA_SHIFT)), which is
            // dBf_fract^2
            d_bf_fract = f_mult_norm(d_bf_fract, d_bf_fract, &mut tmp);

            // Get worst case scaling of multiplication result
            d_bf_e = (DFRACT_BITS - 1 - tmp) - count_leading_bits(d_bf_int);

            // Now multiply integer with fractional part of the result, thus resulting in the
            // overall accurate fractional result
            d_bf_m = f_mult_norm_to(d_bf_int, DFRACT_BITS - 1, d_bf_fract, tmp, d_bf_e);

            my_exp_slot = my_exp_slot.wrapping_add(my_exp);
        } else {
            d_bf_m = 0;
            d_bf_e = 0;
        }

        // Keep the results
        h.d_bf_m[i] = d_bf_m;
        h.d_bf_e[i] = d_bf_e;
    }

    // Make sure that dBf is greater than 1.0 (because it should be a highpass)
    // ...

    0
}

/// `FDKsbrEnc_fastTransientDetect(h_sbrFastTransientDetector, Energies, scaleEnergies,
/// YBufferWriteOffset, tran_vector)`
pub fn fdk_sbr_enc_fast_transient_detect(
    h: &mut FastTranDetector,
    energies: &[&[FixpDbl]],
    scale_energies: &[i32],
    y_buffer_write_offset: i32,
    tran_vector: &mut [u8],
) {
    let n_time_slots = h.n_time_slots as usize;
    let lookahead = h.lookahead as usize;
    let start_band = h.start_band;
    let stop_band = h.stop_band;

    let thr = TRAN_DET_THRSHLD;
    let thr_scale = TRAN_DET_THRSHLD_SCALE;

    // reset transient info
    tran_vector[2] = 0;

    // reset transient candidates
    h.transient_candidates[lookahead..lookahead + n_time_slots].fill(0);

    for time_slot in lookahead..n_time_slots + lookahead {
        let mut norm = 0;
        let mut tmp_e: FixpDbl = 0;
        let mut headroom_en_slot = DFRACT_BITS - 1;

        let mut small_nrg = fl2fxconst_dbl(1e-2f32 as f64);
        let denominator;
        let denominator_scale;

        // determine minimum headroom of energy values for this timeslot
        for band in start_band.max(0) as usize..stop_band.max(0) as usize {
            let tmp_headroom = f_normz(energies[time_slot][band]) - 1;
            if tmp_headroom < headroom_en_slot {
                headroom_en_slot = tmp_headroom;
            }
        }

        let mut i = 0usize;
        for band in start_band.max(0) as usize..stop_band.max(0) as usize {
            // energy is weighted by weightingfactor stored in dBf_m array
            // dBf_m index runs from 0 to stopBand-startband
            // energy shifted by calculated headroom for maximum precision
            let weighted_energy = f_mult(
                energies[time_slot][band].wrapping_shl(headroom_en_slot as u32),
                h.d_bf_m[i],
            );

            // energy is added up
            // shift by 6 to have a headroom for maximum 64 additions
            // shift by dBf_e to handle weighting factor dependent scale factors
            tmp_e =
                tmp_e.wrapping_add(weighted_energy.wrapping_shr((6 + (10 - h.d_bf_e[i])) as u32));
            i += 1;
        }

        // store calculated energy for timeslot
        h.energy_time_slots[time_slot] = tmp_e;

        // calculate overall scale factor for energy of this timeslot
        // =   original scale factor of energies (-scaleEnergies[0]+2*QMF_SCALE_OFFSET or
        //     -scaleEnergies[1]+2*QMF_SCALE_OFFSET depending on YBufferWriteOffset)
        //   + weighting factor scale            (10)
        //   + adding up scale factor            ( 6)
        //   - headroom of energy value          (headroomEnSlot)
        if (time_slot as i32) < y_buffer_write_offset {
            h.energy_time_slots_scale[time_slot] =
                (-scale_energies[0] + 2 * QMF_SCALE_OFFSET) + (10 + 6) - headroom_en_slot;
        } else {
            h.energy_time_slots_scale[time_slot] =
                (-scale_energies[1] + 2 * QMF_SCALE_OFFSET) + (10 + 6) - headroom_en_slot;
        }

        // Add a small energy to the denominator, thus making the transient detection
        // energy-dependent. Loud transients are being detected, silent ones not.

        // make sure that smallNRG does not overflow
        if -h.energy_time_slots_scale[time_slot - 1] + 1 > 5 {
            denominator = small_nrg;
            denominator_scale = 0;
        } else {
            // Leave an additional headroom of 1 bit for this addition.
            small_nrg = scale_value(small_nrg, -(h.energy_time_slots_scale[time_slot - 1] + 1));
            denominator = (h.energy_time_slots[time_slot - 1] >> 1).wrapping_add(small_nrg);
            denominator_scale = h.energy_time_slots_scale[time_slot - 1] + 1;
        }

        h.delta_energy[time_slot] =
            f_div_norm(h.energy_time_slots[time_slot], denominator, &mut norm);
        h.delta_energy_scale[time_slot] =
            h.energy_time_slots_scale[time_slot] - denominator_scale + norm;
    }

    // get transient candidates
    // For every timeslot, check if delta(E) exceeds the threshold. If it did, it could
    // potentially be marked as a transient candidate. However, the 2 slots before the current
    // one must not be transients with an energy higher than 1.4*E(current). If both aren't
    // transients or if the energy of the current timesolot is more than 1.4 times higher than
    // the energy in the last or the one before the last slot, it is marked as a transient.

    fdk_assert!(lookahead >= 2);
    for time_slot in lookahead..n_time_slots + lookahead {
        let energy_cur_slot_weighted =
            f_mult(h.energy_time_slots[time_slot], fl2fxconst_dbl((1.0f32 / 1.4f32) as f64));
        if f_is_less_than(
            h.delta_energy[time_slot],
            h.delta_energy_scale[time_slot],
            thr,
            thr_scale,
        ) == 0
            && ((h.transient_candidates[time_slot - 2] == 0
                && h.transient_candidates[time_slot - 1] == 0)
                || f_is_less_than(
                    energy_cur_slot_weighted,
                    h.energy_time_slots_scale[time_slot],
                    h.energy_time_slots[time_slot - 1],
                    h.energy_time_slots_scale[time_slot - 1],
                ) == 0
                || f_is_less_than(
                    energy_cur_slot_weighted,
                    h.energy_time_slots_scale[time_slot],
                    h.energy_time_slots[time_slot - 2],
                    h.energy_time_slots_scale[time_slot - 2],
                ) == 0)
        {
            // in case of strong transients, subsequent qmf slots might be recognized as
            // transients.
            h.transient_candidates[time_slot] = 1;
        }
    }

    // get transient with max energy
    let mut max_delta_energy: FixpDbl = 0;
    let mut max_delta_energy_scale = 0;
    let mut ind_max = 0;
    let mut is_transient_in_frame = 0;
    for time_slot in 0..n_time_slots {
        let scale = f_max(h.delta_energy_scale[time_slot], max_delta_energy_scale);
        if h.transient_candidates[time_slot] != 0
            && (h.delta_energy[time_slot]
                .wrapping_shr((scale - h.delta_energy_scale[time_slot]) as u32)
                > max_delta_energy.wrapping_shr((scale - max_delta_energy_scale) as u32))
        {
            max_delta_energy = h.delta_energy[time_slot];
            max_delta_energy_scale = scale;
            ind_max = time_slot;
            is_transient_in_frame = 1;
        }
    }

    // from all transient candidates take the one with the biggest energy
    if is_transient_in_frame != 0 {
        tran_vector[0] = ind_max as u8;
        tran_vector[1] = 1;
    } else {
        // reset transient info
        tran_vector[0] = 0;
        tran_vector[1] = 0;
    }

    // check for transients in lookahead
    for time_slot in n_time_slots..n_time_slots + lookahead {
        if h.transient_candidates[time_slot] != 0 {
            tran_vector[2] = 1;
        }
    }

    // update buffers
    for time_slot in 0..lookahead {
        h.transient_candidates[time_slot] = h.transient_candidates[n_time_slots + time_slot];

        // fixpoint stuff
        h.energy_time_slots[time_slot] = h.energy_time_slots[n_time_slots + time_slot];
        h.energy_time_slots_scale[time_slot] = h.energy_time_slots_scale[n_time_slots + time_slot];

        h.delta_energy[time_slot] = h.delta_energy[n_time_slots + time_slot];
        h.delta_energy_scale[time_slot] = h.delta_energy_scale[n_time_slots + time_slot];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u32 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 >> 32) as u32
        }
        fn val(&mut self, kind: u32) -> FixpDbl {
            match kind {
                0 => self.next() as i32,
                1 => [i32::MIN, i32::MAX, 0, -1, 1][(self.next() % 5) as usize],
                _ => (self.next() as i32 & MAXVAL_DBL) >> (self.next() % 32),
            }
        }
    }

    #[test]
    fn transient_columns_vs_straight() {
        let mut rng = Rng(0x0bad_5eed_1234_5678);
        for it in 0..50_000 {
            let kind = rng.next() % 4;
            let e: Vec<FixpDbl> = (0..64).map(|_| rng.val(kind)).collect();
            let thres = rng.val(kind);
            let i_thres = rng.val(kind);
            let j0 = 3 + (rng.next() % 20) as usize;
            let n = (rng.next() % (64 - 3 - j0 as u32 + 1)) as usize;
            let out0: Vec<FixpDbl> = (0..n).map(|_| rng.val(kind)).collect();
            let mut straight = out0.clone();
            for (c, t) in straight.iter_mut().enumerate() {
                let j = j0 + c;
                let mut delta: FixpDbl = 0;
                let mut tran: FixpDbl = 0;
                for d in 1..4usize {
                    delta = delta.wrapping_add(e[j + d]);
                    delta = delta.wrapping_sub(e[j - d]);
                    delta = delta.wrapping_sub(thres);
                    if delta > 0 {
                        tran = f_mult_add_div2(tran, i_thres, delta);
                    }
                }
                *t = t.wrapping_add(tran << 1);
            }
            let mut fast = out0;
            transient_columns(&e, j0, thres, i_thres, &mut fast);
            assert_eq!(straight, fast, "{it}");
        }
    }
}
