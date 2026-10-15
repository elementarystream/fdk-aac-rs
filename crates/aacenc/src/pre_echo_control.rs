// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACenc/src/pre_echo_control.h, libAACenc/src/pre_echo_control.cpp.

//! Port of `pre_echo_control.h`: pre echo control.
//!
//! The `aarch64` and `x86` translation units of `pre_echo_control.cpp` are identical apart from
//! `FDK_ASSERT`; the variants differ only in `fMult`.
//!
//! The shifts by `scaling` and `scaling + 1` (twice the difference of two MDCT exponents) are
//! undefined in C for amounts outside `0..=31`; both reference builds compile them as scalar
//! shifts with a register amount (`asr`/`lsl` on arm64, `sarl`/`shll %cl` on x86), which take the
//! amount modulo 32, as the port does. The integer products wrap.

use crate::psy_configuration::PCM_QUANT_THR_SCALE;
use fdk_aac_rs_fdk::common_fix::*;
use fdk_aac_rs_fdk::fdk_assert;

/// `FDKaacEnc_InitPreEchoControl()`
pub fn fdk_aac_enc_init_pre_echo_control(
    pb_threshold_nm1: &mut [FixpDbl],
    calc_pre_echo: &mut i32,
    num_pb: i32,
    sfb_pcm_quant_threshold: &[FixpDbl],
    mdct_scalenm1: &mut i32,
) {
    *mdct_scalenm1 = PCM_QUANT_THR_SCALE >> 1;

    let n = num_pb.max(0) as usize;
    pb_threshold_nm1[..n].copy_from_slice(&sfb_pcm_quant_threshold[..n]);

    *calc_pre_echo = 1;
}

/// `FDKaacEnc_PreEchoControl()`
#[allow(clippy::too_many_arguments)]
pub fn fdk_aac_enc_pre_echo_control(
    pb_threshold_nm1: &mut [FixpDbl],
    calc_pre_echo: i32,
    num_pb: i32,
    max_allowed_increase_factor: i32,
    min_remaining_threshold_factor: FixpSgl,
    pb_threshold: &mut [FixpDbl],
    mdct_scale: i32,
    mdct_scalenm1: &mut i32,
) {
    let n = num_pb.max(0) as usize;

    // If lastWindowSequence in previous frame was start- or stop-window, skip preechocontrol
    // calculation
    if calc_pre_echo == 0 {
        // copy thresholds to internal memory
        pb_threshold_nm1[..n].copy_from_slice(&pb_threshold[..n]);
        *mdct_scalenm1 = mdct_scale;
        return;
    }

    if mdct_scale > *mdct_scalenm1 {
        // if current thresholds are downscaled more than the ones from the last block
        let scaling: i32 = mdct_scale.wrapping_sub(*mdct_scalenm1).wrapping_mul(2);
        for i in 0..n {
            // multiplication with return data type fract ist equivalent to int multiplication
            fdk_assert!(scaling >= 0);
            let tmp_threshold1: FixpDbl = max_allowed_increase_factor
                .wrapping_mul(pb_threshold_nm1[i].wrapping_shr(scaling as u32));
            let tmp_threshold2: FixpDbl = f_mult(min_remaining_threshold_factor, pb_threshold[i]);

            let mut tmp: FixpDbl = pb_threshold[i];

            // copy thresholds to internal memory
            pb_threshold_nm1[i] = tmp;

            tmp = fix_min(tmp, tmp_threshold1);
            pb_threshold[i] = fix_max(tmp, tmp_threshold2);
        }
    } else {
        // if thresholds of last block are more downscaled than the current ones
        let scaling: i32 = (*mdct_scalenm1).wrapping_sub(mdct_scale).wrapping_mul(2);
        for i in 0..n {
            // multiplication with return data type fract ist equivalent to int multiplication
            let tmp_threshold1: FixpDbl =
                (max_allowed_increase_factor >> 1).wrapping_mul(pb_threshold_nm1[i]);
            let tmp_threshold2: FixpDbl = f_mult(min_remaining_threshold_factor, pb_threshold[i]);

            // copy thresholds to internal memory
            pb_threshold_nm1[i] = pb_threshold[i];

            fdk_assert!(scaling >= 0);
            let s = scaling.wrapping_add(1) as u32;
            if pb_threshold[i].wrapping_shr(s) > tmp_threshold1 {
                pb_threshold[i] = tmp_threshold1.wrapping_shl(s);
            }
            pb_threshold[i] = fix_max(pb_threshold[i], tmp_threshold2);
        }
    }

    *mdct_scalenm1 = mdct_scale;
}
