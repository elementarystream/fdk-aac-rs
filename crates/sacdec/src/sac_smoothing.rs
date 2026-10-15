// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libSACdec/src/sac_smoothing.h, libSACdec/src/sac_smoothing.cpp.

//! Port of `sac_smoothing.h` and `sac_smoothing.cpp`: the smoothing of the M2 parameters and of
//! the OPD phases.

use fdk_aac_rs_fdk::common_fix::*;

use crate::sac_dec::*;
use crate::sac_dec_interface::*;
use crate::sac_rom::*;

/// `calcFilterCoeff__FDK(self, ps, frame)`
fn calc_filter_coeff__fdk(self_: &SpatialDec, ps: i32, frame: &SpatialBsFrame) -> FixpDbl {
    let mut d_slots = param_slot_at(frame, ps).wrapping_sub(self_.smooth_state.prev_param_slot);

    if d_slots <= 0 {
        d_slots = d_slots.wrapping_add(self_.time_slots);
    }

    f_div_norm_noexp(d_slots, smg_time_at(self_, ps))
}

/// `frame->paramSlot[ps]` (`ps` is in 0..=8 in every call; deviation: other values read 0).
#[inline]
pub(crate) fn param_slot_at(frame: &SpatialBsFrame, ps: i32) -> i32 {
    frame.param_slot.get(ps as usize).copied().unwrap_or(0)
}

/// `self->smgTime[ps]` (deviation as [`param_slot_at`]).
#[inline]
fn smg_time_at(self_: &SpatialDec, ps: i32) -> i32 {
    self_.smg_time.get(ps as usize).copied().unwrap_or(0)
}

/// `getSmoothOnOff(self, ps, pb)`
fn get_smooth_on_off(self_: &SpatialDec, ps: i32, pb: usize) -> i32 {
    self_.smg_data.get(ps as usize).map_or(0, |r| r[pb] as i32)
}

/// `SpatialDecSmoothM1andM2(self, frame, ps)`
pub fn spatial_dec_smooth_m1_and_m2(self_: &mut SpatialDec, frame: &SpatialBsFrame, ps: i32) {
    let mut residual_bands = 0;

    if self_.residual_coding != 0 {
        let boxes = (self_.num_ott_boxes.max(0) as usize).min(MAX_NUM_OTT);
        for i in 0..boxes {
            if self_.residual_bands[i] as i32 > residual_bands {
                residual_bands = self_.residual_bands[i] as i32;
            }
        }
    }

    let delta__fdk = calc_filter_coeff__fdk(self_, ps, frame);

    let one_minus_delta__fdk = if delta__fdk == MAXVAL_DBL {
        // FL2FXCONST_DBL(1.0f)
        0
    } else if delta__fdk == 0 {
        MAXVAL_DBL // FL2FXCONST_DBL(1.0f)
    } else {
        (fl2fxconst_dbl(0.5) - (delta__fdk >> 1)).wrapping_shl(1)
    };

    let rows = (self_.num_m2rows.max(0) as usize).min(NUM_OUT_CH);
    let cols = (self_.num_v_channels.max(0) as usize).min(NUM_V_CH);
    for pb in 0..(self_.num_parameter_bands.max(0) as usize).min(MAX_PARAMETER_BANDS) {
        let smooth_band = get_smooth_on_off(self_, ps, pb);
        if smooth_band != 0 && pb as i32 >= residual_bands {
            for row in 0..rows {
                for col in 0..cols {
                    self_.m2_real__fdk[row][col][pb] =
                        (f_mult_div2(delta__fdk, self_.m2_real__fdk[row][col][pb]).wrapping_add(
                            f_mult_div2(
                                one_minus_delta__fdk,
                                self_.m2_real_prev__fdk[row][col][pb],
                            ),
                        ))
                        .wrapping_shl(1);
                    if self_.phase_coding == 3 {
                        self_.m2_imag__fdk[row][col][pb] =
                            (f_mult_div2(delta__fdk, self_.m2_imag__fdk[row][col][pb])
                                .wrapping_add(f_mult_div2(
                                    one_minus_delta__fdk,
                                    self_.m2_imag_prev__fdk[row][col][pb],
                                )))
                            .wrapping_shl(1);
                    }
                }
            }
        }
    }
    self_.smooth_state.prev_param_slot = param_slot_at(frame, ps);
}

/// `initParameterSmoothing(self)`: init states
pub fn init_parameter_smoothing(self_: &mut SpatialDec) {
    self_.smooth_state.prev_param_slot = 0;
}

/// `SpatialDecSmoothOPD(self, frame, ps)`
pub fn spatial_dec_smooth_opd(self_: &mut SpatialDec, frame: &SpatialBsFrame, ps: i32) {
    let quant_coarse =
        frame.ipd_lossless_data[0].bs_quant_coarse_xxx.get(ps as usize).copied().unwrap_or(0)
            as i32;
    let npb = (self_.num_parameter_bands.max(0) as usize).min(MAX_PARAMETER_BANDS);

    if frame.opd_smoothing_mode == 0 {
        self_.smooth_state.opd_left_state__fdk[..npb]
            .copy_from_slice(&self_.phase_left__fdk[..npb]);
        self_.smooth_state.opd_right_state__fdk[..npb]
            .copy_from_slice(&self_.phase_right__fdk[..npb]);
    } else {
        let d_slots = if ps == 0 {
            param_slot_at(frame, ps) + 1
        } else {
            param_slot_at(frame, ps) - param_slot_at(frame, ps - 1)
        };

        let delta__fdk: FixpDbl = (fl2fxconst_dbl(0.0078125)).wrapping_mul(d_slots);

        let one_minus_delta__fdk = if delta__fdk == MAXVAL_DBL {
            0
        } else if delta__fdk == 0 {
            MAXVAL_DBL
        } else {
            (fl2fxconst_dbl(0.5) - (delta__fdk >> 1)).wrapping_shl(1)
        };

        let two_pi = PI__IPD << 1;
        let left = &mut self_.smooth_state.opd_left_state__fdk;
        let right = &mut self_.smooth_state.opd_right_state__fdk;
        for pb in 0..npb {
            let mut tmp_l = self_.phase_left__fdk[pb];
            let mut tmp_r = self_.phase_right__fdk[pb];

            // The loops terminate: the phases and the smoothed states stay within a few
            // multiples of 2*pi (the bounds below cannot overflow for them).
            while tmp_l > left[pb].wrapping_add(PI__IPD) {
                tmp_l = tmp_l.wrapping_sub(two_pi);
            }
            while tmp_l < left[pb].wrapping_sub(PI__IPD) {
                tmp_l = tmp_l.wrapping_add(two_pi);
            }
            while tmp_r > right[pb].wrapping_add(PI__IPD) {
                tmp_r = tmp_r.wrapping_sub(two_pi);
            }
            while tmp_r < right[pb].wrapping_sub(PI__IPD) {
                tmp_r = tmp_r.wrapping_add(two_pi);
            }

            left[pb] =
                f_mult(delta__fdk, tmp_l).wrapping_add(f_mult(one_minus_delta__fdk, left[pb]));
            right[pb] =
                f_mult(delta__fdk, tmp_r).wrapping_add(f_mult(one_minus_delta__fdk, right[pb]));

            let mut tmp = (((tmp_l >> 1).wrapping_sub(tmp_r >> 1))
                .wrapping_sub((left[pb] >> 1).wrapping_sub(right[pb] >> 1)))
            .wrapping_shl(1);
            while tmp > PI__IPD {
                tmp = tmp.wrapping_sub(two_pi);
            }
            while tmp < -PI__IPD {
                tmp = tmp.wrapping_add(two_pi);
            }
            if fixp_abs(tmp)
                > f_mult(
                    if quant_coarse != 0 {
                        fl2fxconst_dbl((50.0f32 / 180.0f32) as f64)
                    } else {
                        fl2fxconst_dbl((25.0f32 / 180.0f32) as f64)
                    },
                    PI__IPD,
                )
            {
                left[pb] = tmp_l;
                right[pb] = tmp_r;
            }

            while left[pb] > two_pi {
                left[pb] = left[pb].wrapping_sub(two_pi);
            }
            while left[pb] < 0 {
                left[pb] = left[pb].wrapping_add(two_pi);
            }
            while right[pb] > two_pi {
                right[pb] = right[pb].wrapping_sub(two_pi);
            }
            while right[pb] < 0 {
                right[pb] = right[pb].wrapping_add(two_pi);
            }

            self_.phase_left__fdk[pb] = left[pb];
            self_.phase_right__fdk[pb] = right[pb];
        }
    }
}
