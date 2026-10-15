// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libSACenc/src/sacenc_onsetdetect.h, libSACenc/src/sacenc_onsetdetect.cpp.

//! Port of `sacenc_onsetdetect.h` and `sacenc_onsetdetect.cpp`: the transient (onset)
//! detection.
//!
//! # Variants
//!
//! The code is identical in the `aarch64` and `x86` builds; the results differ through
//! `fMultDiv2`. The loops that both builds vectorize (the common scaling of the energies)
//! shift by amounts of 0..=31, where the vector shifts equal the scalar ones.
//!
//! # Quirks
//!
//! `fdk_sacenc_onsetDetect_Init` checks the new configuration against the maximum number of
//! time slots and the lower bound of the previous initialization (the values of the instance
//! before they are overwritten): a second initialization with more time slots than the first
//! fails. The port keeps this.

use fdk_aac_rs_fdk::common_fix::*;

use crate::sacenc_bitstream::{MAX_NUM_PARAMS, MAX_TIME_SLOTS};
use crate::sacenc_const::SACENC_FLOAT_EPSILON;
use crate::sacenc_lib::*;
use crate::sacenc_vectorfunctions::*;

/// `MAX_NUM_TRANS`
pub const MAX_NUM_TRANS: usize = MAX_NUM_PARAMS / 2;

/// `SPACE_ONSET_THRESHOLD`
const SPACE_ONSET_THRESHOLD: f64 = 3.0;
/// `SPACE_ONSET_THRESHOLD_SF`
const SPACE_ONSET_THRESHOLD_SF: i32 = 3;
/// `SPACE_ONSET_THRESHOLD_SQUARE`: `FL2FXCONST_DBL((1.0 / (SPACE_ONSET_THRESHOLD *
/// SPACE_ONSET_THRESHOLD)) * (float)(1 << SPACE_ONSET_THRESHOLD_SF))` (computed in double)
const SPACE_ONSET_THRESHOLD_SQUARE: FixpDbl = fl2fxconst_dbl(
    (1.0 / (SPACE_ONSET_THRESHOLD * SPACE_ONSET_THRESHOLD))
        * ((1 << SPACE_ONSET_THRESHOLD_SF) as f32) as f64,
);

/// `ONSET_DETECT_CONFIG`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OnsetDetectConfig {
    pub max_time_slots: i32,
    /// calc transient detection in ]lowerBoundOnsetDetection; upperBoundOnsetDetection[
    pub lower_bound_onset_detection: i32,
    pub upper_bound_onset_detection: i32,
}

/// `struct ONSET_DETECT`
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OnsetDetect {
    pub max_time_slots: i32,
    pub min_transient_distance: i32,
    pub avg_energy_distance: i32,
    pub lower_bound_onset_detection: i32,
    pub upper_bound_onset_detection: i32,
    /// `pEnergyHist__FDK`: `16 + maxTimeSlots` values (of the open call)
    pub p_energy_hist_fdk: Vec<FixpDbl>,
    /// `pEnergyHistScale`: `16 + maxTimeSlots` values (of the open call)
    pub p_energy_hist_scale: Vec<i8>,
    pub avg_energy_distance_scale: i8,
}

/// `HANDLE_ONSET_DETECT`
pub type HandleOnsetDetect = Option<Box<OnsetDetect>>;

/// `fdk_sacenc_onsetDetect_Open(phOnset, maxTimeSlots)`
pub fn fdk_sacenc_onset_detect_open(
    ph_onset: Option<&mut HandleOnsetDetect>,
    max_time_slots: u32,
) -> FdkSacencError {
    match ph_onset {
        None => SACENC_INVALID_HANDLE,
        Some(h) => {
            let n = 16usize.wrapping_add(max_time_slots as usize);
            *h = Some(Box::new(OnsetDetect {
                p_energy_hist_fdk: vec![0; n],
                p_energy_hist_scale: vec![0; n],
                max_time_slots: max_time_slots as i32,
                // minimum distance between detected transients
                min_transient_distance: 8,
                // average energy distance
                avg_energy_distance: 16,
                avg_energy_distance_scale: 4,
                ..Default::default()
            }));
            SACENC_OK
        }
    }
}

/// `fdk_sacenc_onsetDetect_Init(hOnset, pOnsetDetectConfig, initFlags)`
pub fn fdk_sacenc_onset_detect_init(
    h_onset: &mut OnsetDetect,
    p_onset_detect_config: &OnsetDetectConfig,
    init_flags: u32,
) -> FdkSacencError {
    if (p_onset_detect_config.max_time_slots > h_onset.max_time_slots)
        || (p_onset_detect_config.upper_bound_onset_detection < h_onset.lower_bound_onset_detection)
    {
        return SACENC_INVALID_CONFIG;
    }

    h_onset.max_time_slots = p_onset_detect_config.max_time_slots;
    h_onset.lower_bound_onset_detection = p_onset_detect_config.lower_bound_onset_detection;
    h_onset.upper_bound_onset_detection = p_onset_detect_config.upper_bound_onset_detection;

    // minimum distance between detected transients
    h_onset.min_transient_distance = 8;
    // average energy distance
    h_onset.avg_energy_distance = 16;

    h_onset.avg_energy_distance_scale = 4;

    // Init / Reset
    if init_flags != 0 {
        let n = (h_onset.avg_energy_distance + h_onset.max_time_slots).max(0) as usize;
        for v in h_onset.p_energy_hist_scale.iter_mut().take(n) {
            *v = -(DFRACT_BITS - 3) as i8;
        }

        let e = fl2fxconst_dbl((SACENC_FLOAT_EPSILON * (1i32 << (DFRACT_BITS - 3)) as f32) as f64);
        for v in h_onset.p_energy_hist_fdk.iter_mut().take(n) {
            *v = e;
        }
    }

    SACENC_OK
}

/// `fdk_sacenc_onsetDetect_Close(phOnset)`
pub fn fdk_sacenc_onset_detect_close(ph_onset: Option<&mut HandleOnsetDetect>) -> FdkSacencError {
    if let Some(h) = ph_onset {
        *h = None;
    }
    SACENC_OK
}

/// `fdk_sacenc_onsetDetect_Update(hOnset, timeSlots)`: shifts the energy history by
/// `timeSlots`.
pub fn fdk_sacenc_onset_detect_update(
    h_onset: &mut OnsetDetect,
    time_slots: i32,
) -> FdkSacencError {
    if time_slots > h_onset.max_time_slots {
        return SACENC_INVALID_CONFIG;
    }
    let m = h_onset.avg_energy_distance.max(0) as usize;
    let t = time_slots.max(0) as usize;
    // Shift old data
    for i in 0..m {
        h_onset.p_energy_hist_fdk[i] = h_onset.p_energy_hist_fdk[i + t];
        h_onset.p_energy_hist_scale[i] = h_onset.p_energy_hist_scale[i + t];
    }

    // Clear for new data
    let e = fl2fxconst_dbl(SACENC_FLOAT_EPSILON as f64);
    for v in h_onset.p_energy_hist_fdk[m..m + t].iter_mut() {
        *v = e;
    }
    SACENC_OK
}

/// `fdk_sacenc_onsetDetect_Apply(hOnset, nTimeSlots, nHybridBands, ppHybridData__FDK,
/// hybridDataScale, prevPos, pTransientPos)`: detects a transient in the time slots
/// `ppHybridData__FDK[0..nTimeSlots]`.
pub fn fdk_sacenc_onset_detect_apply<R: AsRef<[FixpDpk]>>(
    h_onset: &mut OnsetDetect,
    n_time_slots: i32,
    n_hybrid_bands: i32,
    pp_hybrid_data_fdk: &[R],
    hybrid_data_scale: i32,
    prev_pos: i32,
    p_transient_pos: &mut [i32; MAX_NUM_TRANS],
) -> FdkSacencError {
    let mut envs = [0 as FixpDbl; 16 + MAX_TIME_SLOTS];

    if (n_time_slots < 0)
        || (n_time_slots > h_onset.max_time_slots)
        || (h_onset.lower_bound_onset_detection < -1)
        || (h_onset.upper_bound_onset_detection > n_hybrid_bands)
    {
        return SACENC_INVALID_CONFIG;
    }

    let lower_bound_onset_detection = h_onset.lower_bound_onset_detection;
    let upper_bound_onset_detection = h_onset.upper_bound_onset_detection;
    let m = h_onset.avg_energy_distance;
    let nts = n_time_slots;

    let threshold_square = SPACE_ONSET_THRESHOLD_SQUARE;
    let mut tr_cnt = 0usize;

    // reset transient array
    p_transient_pos.fill(-1);

    // minimum transient distance of minTransDist QMF samples
    let mut curr_pos = if prev_pos > 0 {
        fix_max(nts, prev_pos - nts + h_onset.min_transient_distance)
    } else {
        nts
    };

    // get energy and scalefactor for each time slot
    let in_scale = 3; // scale factor determined empirically
    let env = &mut h_onset.p_energy_hist_fdk;
    let env_scale = &mut h_onset.p_energy_hist_scale;
    for ts in 0..nts as usize {
        let mut out_scale = 0;
        let row = pp_hybrid_data_fdk[ts].as_ref();
        env[m as usize + ts] = sum_up_cplx_pow2(
            &row[(lower_bound_onset_detection + 1) as usize..],
            SUM_UP_DYNAMIC_SCALE,
            in_scale,
            &mut out_scale,
            upper_bound_onset_detection - lower_bound_onset_detection - 1,
        );
        // SCHAR assignment: truncated to 8 bits.
        env_scale[m as usize + ts] =
            out_scale.wrapping_add(hybrid_data_scale.wrapping_shl(1)) as i8;
    }

    let n = (nts + m) as usize;

    // calculate common scale for all time slots
    let mut max_scale: i8 = -(DFRACT_BITS - 1) as i8;
    for &s in &env_scale[..n] {
        max_scale = fix_max(max_scale, s);
    }

    // apply common scale and store energy in temporary buffer
    for i in 0..n {
        envs[i] = env[i] >> fix_min(max_scale as i32 - env_scale[i] as i32, DFRACT_BITS - 1);
    }

    let mut max_val: FixpDbl = 0;
    for &e in &envs[..n] {
        max_val |= f_abs(e);
    }

    let s = fix_max(0, cnt_leading_zeros(max_val) - 1);

    for e in &mut envs[..n] {
        *e <<= s;
    }

    let sc = h_onset.avg_energy_distance_scale as i32;
    let mut curr_pos_prev = curr_pos;
    let mut p2: FixpDbl = 0;
    while (curr_pos < (nts << 1)) && (tr_cnt < MAX_NUM_TRANS) {
        let p1 = f_mult_div2(envs[(curr_pos - nts + m) as usize], threshold_square)
            >> (SPACE_ONSET_THRESHOLD_SF - 1);

        // Calculate average of past M energy values
        if curr_pos_prev == (curr_pos - 1) {
            // remove last and add new element
            p2 = p2.wrapping_sub(envs[(curr_pos_prev - nts) as usize] >> sc);
            p2 = p2.wrapping_add(envs[(curr_pos - nts + m - 1) as usize] >> sc);
        } else {
            // calculate complete vector
            p2 = 0;
            for ts in 0..m {
                p2 = p2.wrapping_add(envs[(curr_pos - nts + ts) as usize] >> sc);
            }
        }
        curr_pos_prev = curr_pos;

        // save position if transient found
        if p1 > p2 {
            p_transient_pos[tr_cnt] = curr_pos;
            tr_cnt += 1;
            curr_pos += h_onset.min_transient_distance;
        }
        curr_pos += 1;
    }

    SACENC_OK
}
