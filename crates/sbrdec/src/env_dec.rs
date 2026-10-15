// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libSBRdec/src/env_dec.h, libSBRdec/src/env_dec.cpp.

//! Port of `env_dec.h` and `env_dec.cpp`: envelope decoding (delta decoding, requantization,
//! coupling unmapping) and the error concealment of the envelope and noise floor data. The main
//! entry point is [`decode_sbr_data`].
//!
//! The values are `FIXP_SGL`/`SCHAR`; the C code computes in `int` and truncates when it stores
//! (the port uses `wrapping_*` on the narrow types).

use crate::env_extr::*;
use crate::lpp_tran::{MAX_FREQ_COEFFS, MAX_INVF_BANDS};
use crate::transcendent::*;
use fdk_aac_rs_fdk::common_fix::*;
use fdk_aac_rs_fdk::fdk_assert;

/// `SBR_ENERGY_PAN_OFFSET`
const SBR_ENERGY_PAN_OFFSET: FixpSgl = (12 << ENV_EXP_FRACT) as FixpSgl;
/// `SBR_MAX_ENERGY`
const SBR_MAX_ENERGY: FixpSgl = (35 << ENV_EXP_FRACT) as FixpSgl;
/// `DECAY`
const DECAY: FixpSgl = (1 << ENV_EXP_FRACT) as FixpSgl;
/// `DECAY_COUPLING`: if the energy data is not shifted, use 1 instead of 0.5
const DECAY_COUPLING: FixpSgl = 1;

/// `indexLow2High(offset, index, res)`: converts a table index.
fn index_low2_high(offset: i32, index: i32, res: i32) -> i32 {
    if res == 0 {
        if offset >= 0 {
            if index < offset {
                index
            } else {
                2 * index - offset
            }
        } else {
            let offset = -offset;
            if index < offset {
                2 * index + index
            } else {
                2 * index + offset
            }
        }
    } else {
        index
    }
}

/// Writes `prevData[idx]`; the indices are inside `sfb_nrg_prev` for consistent frequency
/// tables (`offset = 2 * nSfb[0] - nSfb[1]`), others are ignored.
#[inline]
fn set_prev(prev_data: &mut [FixpSgl; MAX_FREQ_COEFFS], idx: i32, v: FixpSgl) {
    if let Some(p) = prev_data.get_mut(idx as usize) {
        *p = v;
    }
}

/// `mapLowResEnergyVal(currVal, prevData, offset, index, res)`: updates the previous envelope
/// value for delta coding (always stored with the high frequency resolution).
fn map_low_res_energy_val(
    curr_val: FixpSgl,
    prev_data: &mut [FixpSgl; MAX_FREQ_COEFFS],
    offset: i32,
    index: i32,
    res: i32,
) {
    if res == 0 {
        if offset >= 0 {
            if index < offset {
                set_prev(prev_data, index, curr_val);
            } else {
                set_prev(prev_data, 2 * index - offset, curr_val);
                set_prev(prev_data, 2 * index + 1 - offset, curr_val);
            }
        } else {
            let offset = -offset;
            if index < offset {
                set_prev(prev_data, 3 * index, curr_val);
                set_prev(prev_data, 3 * index + 1, curr_val);
                set_prev(prev_data, 3 * index + 2, curr_val);
            } else {
                set_prev(prev_data, 2 * index + offset, curr_val);
                set_prev(prev_data, 2 * index + 1 + offset, curr_val);
            }
        }
    } else {
        set_prev(prev_data, index, curr_val);
    }
}

/// `decodeSbrData()`: converts the raw envelope and noise floor data to energy levels, with
/// concealment in case of errors.
pub fn decode_sbr_data(
    h_header_data: &mut SbrHeaderData,
    h_data_left: &mut SbrFrameData,
    h_prev_data_left: &mut SbrPrevFrameData,
    h_data_right: Option<&mut SbrFrameData>,
    h_prev_data_right: Option<&mut SbrPrevFrameData>,
) {
    // Save previous energy values to be able to reuse them later for concealment.
    let temp_sfb_nrg_prev = h_prev_data_left.sfb_nrg_prev;

    if h_header_data.frame_error_flag != 0 || h_header_data.bs_info.pvc_mode == 0 {
        decode_envelope(h_header_data, h_data_left, h_prev_data_left, h_prev_data_right.as_deref());
    } else {
        fdk_assert!(h_data_right.is_none());
    }
    decode_noise_floorlevels(h_header_data, h_data_left, h_prev_data_left);

    if let Some(h_data_right) = h_data_right {
        // The right channel always has previous frame data in the decoder.
        let Some(h_prev_data_right) = h_prev_data_right else {
            return;
        };
        let err_left = h_header_data.frame_error_flag;
        decode_envelope(h_header_data, h_data_right, h_prev_data_right, Some(&*h_prev_data_left));
        decode_noise_floorlevels(h_header_data, h_data_right, h_prev_data_right);

        if err_left == 0 && h_header_data.frame_error_flag != 0 {
            // If an error occurs in the right channel where the left channel seemed ok, we apply
            // concealment also on the left channel. This ensures that the coupling modes of both
            // channels match and that we have the same number of envelopes in coupling mode.
            // However, as the left channel has already been processed before, the resulting
            // energy levels are not the same as if the left channel had been concealed during
            // the first call of decodeEnvelope().
            // Restore previous energy values for concealment, because the values have been
            // overwritten by the first call of decodeEnvelope().
            h_prev_data_left.sfb_nrg_prev = temp_sfb_nrg_prev;
            // Do concealment
            decode_envelope(
                h_header_data,
                h_data_left,
                h_prev_data_left,
                Some(&*h_prev_data_right),
            );
        }

        if h_data_left.coupling != 0 {
            sbr_envelope_unmapping(h_header_data, h_data_left, h_data_right);
        }
    }
}

/// `sbr_envelope_unmapping()`: converts from coupled channels to independent L/R data.
fn sbr_envelope_unmapping(
    h_header_data: &SbrHeaderData,
    h_data_left: &mut SbrFrameData,
    h_data_right: &mut SbrFrameData,
) {
    // 1. Unmap (already dequantized) coupled envelope energies
    let n = (h_data_left.n_scale_factors.max(0) as usize).min(h_data_left.i_envelope.len());
    for i in 0..n {
        let r = h_data_right.i_envelope[i] as i32;
        let l = h_data_left.i_envelope[i] as i32;
        let temp_r_m = (r & MASK_M) as FixpSgl;
        let mut temp_r_e = (r & MASK_E) as i8;

        // -18 = ld(UNMAPPING_SCALE / h_data_right->nChannels)
        temp_r_e = (temp_r_e as i32 - (18 + NRG_EXP_OFFSET)) as i8;
        let temp_l_m = (l & MASK_M) as FixpSgl;
        let mut temp_l_e = (l & MASK_E) as i8;

        temp_l_e = (temp_l_e as i32 - NRG_EXP_OFFSET) as i8;

        // Calculate tempRight+1
        let (temp_rplus1_m, temp_rplus1_e) =
            fdk_add_mant_exp_sgl(temp_r_m, temp_r_e, fl2fxconst_sgl(0.5f32 as f64), 1); // 1.0

        // 2 * tempLeft
        let (mut new_r_m, mut new_r_e) = fdk_divide_mant_exp_sgl(
            temp_l_m,
            (temp_l_e as i32 + 1) as i8,
            temp_rplus1_m,
            temp_rplus1_e,
        );

        if new_r_m as i32 >= (MAXVAL_SGL - ROUNDING as i32) {
            new_r_m >>= 1;
            new_r_e = new_r_e.wrapping_add(1);
        }

        let new_l_m = fx_dbl2fx_sgl(f_mult(temp_r_m, new_r_m));
        let new_l_e = (temp_r_e as i32 + new_r_e as i32) as i8;

        h_data_right.i_envelope[i] = pack(new_r_m, new_r_e as i32 + NRG_EXP_OFFSET);
        h_data_left.i_envelope[i] = pack(new_l_m, new_l_e as i32 + NRG_EXP_OFFSET);
    }

    // 2. Dequantize and unmap coupled noise floor levels
    let n = (h_header_data.freq_band_data.n_nfb as usize
        * h_data_left.frame_info.n_noise_envelopes as usize)
        .min(h_data_left.sbr_noise_floor_level.len());
    for i in 0..n {
        let temp_l_e = (6 - h_data_left.sbr_noise_floor_level[i] as i32) as i8;
        let temp_r_e = (h_data_right.sbr_noise_floor_level[i] as i32 - 12) as i8; // SBR_ENERGY_PAN_OFFSET

        // Calculate tempR+1
        let (temp_rplus1_m, temp_rplus1_e) = fdk_add_mant_exp_sgl(
            fl2fxconst_sgl(0.5f32 as f64),
            (1 + temp_r_e as i32) as i8, // tempR
            fl2fxconst_sgl(0.5f32 as f64),
            1, // 1.0
        );

        // Calculate 2*tempLeft/(tempR+1)
        let (new_r_m, new_r_e) = fdk_divide_mant_exp_sgl(
            fl2fxconst_sgl(0.5f32 as f64),
            (temp_l_e as i32 + 2) as i8, // 2 * tempLeft
            temp_rplus1_m,
            temp_rplus1_e,
        );

        // L = tempR * R
        let new_l_m = new_r_m;
        let new_l_e = (new_r_e as i32 + temp_r_e as i32) as i8;
        h_data_right.sbr_noise_floor_level[i] = pack(new_r_m, new_r_e as i32 + NOISE_EXP_OFFSET);
        h_data_left.sbr_noise_floor_level[i] = pack(new_l_m, new_l_e as i32 + NOISE_EXP_OFFSET);
    }
}

/// `((FIXP_SGL)((SHORT)(FIXP_SGL)(m + ROUNDING) & MASK_M)) + (FIXP_SGL)((SHORT)(FIXP_SGL)e &
/// MASK_E)`: the pseudo float of a rounded mantissa and an exponent.
#[inline]
fn pack(m: FixpSgl, e: i32) -> FixpSgl {
    let mant = ((m as i32 + ROUNDING as i32) as i16 as i32 & MASK_M) as FixpSgl;
    let exp = (e as i16 as i32 & MASK_E) as FixpSgl;
    (mant as i32 + exp as i32) as FixpSgl
}

/// `leanSbrConcealment()`: if the real frame info is not available due to a frame loss, a
/// replacement is constructed with 1 envelope spanning the whole frame (FIX-FIX). The delta coded
/// energies are set to negative values, resulting in a fade-down. In case of coupling, the
/// balance channel moves towards the center.
fn lean_sbr_concealment(
    h_header_data: &SbrHeaderData,
    h_sbr_data: &mut SbrFrameData,
    h_prev_data: &SbrPrevFrameData,
) {
    let current_start_pos =
        f_max(0, h_prev_data.stop_pos as i32 - h_header_data.number_time_slots as i32);
    let current_stop_pos = h_header_data.number_time_slots as i32;

    // Use some settings of the previous frame
    h_sbr_data.amp_resolution_current_frame = h_prev_data.amp_res as i32;
    h_sbr_data.coupling = h_prev_data.coupling;
    for i in 0..MAX_INVF_BANDS {
        h_sbr_data.sbr_invf_mode[i] = h_prev_data.sbr_invf_mode[i];
    }

    // Generate concealing control data
    h_sbr_data.frame_info.n_envelopes = 1;
    h_sbr_data.frame_info.borders[0] = current_start_pos as u8;
    h_sbr_data.frame_info.borders[1] = current_stop_pos as u8;
    h_sbr_data.frame_info.freq_res[0] = 1;
    h_sbr_data.frame_info.tran_env = -1; // no transient
    h_sbr_data.frame_info.n_noise_envelopes = 1;
    h_sbr_data.frame_info.borders_noise[0] = current_start_pos as u8;
    h_sbr_data.frame_info.borders_noise[1] = current_stop_pos as u8;

    h_sbr_data.n_scale_factors = h_header_data.freq_band_data.n_sfb[1] as i32;

    // Generate fake envelope data
    h_sbr_data.domain_vec[0] = 1;

    let (mut target, mut step) = if h_sbr_data.coupling == COUPLING_BAL {
        (SBR_ENERGY_PAN_OFFSET, DECAY_COUPLING)
    } else {
        (0 as FixpSgl, DECAY)
    };
    if h_header_data.bs_info.amp_resolution == 0 {
        target <<= 1;
        step <<= 1;
    }

    for i in 0..(h_sbr_data.n_scale_factors as usize).min(MAX_FREQ_COEFFS) {
        if h_prev_data.sfb_nrg_prev[i] > target {
            h_sbr_data.i_envelope[i] = step.wrapping_neg();
        } else {
            h_sbr_data.i_envelope[i] = step;
        }
    }

    // Noisefloor levels are always cleared ...
    h_sbr_data.domain_vec_noise[0] = 1;
    h_sbr_data.sbr_noise_floor_level.fill(0);

    // ... and so are the sines
    h_sbr_data.add_harmonics.fill(0);
}

/// `decodeEnvelope()`: builds the reference energies and noise levels from the bitstream
/// elements. `other_channel` is the other channel's previous frame data (`NULL` for mono).
fn decode_envelope(
    h_header_data: &mut SbrHeaderData,
    h_sbr_data: &mut SbrFrameData,
    h_prev_data: &mut SbrPrevFrameData,
    other_channel: Option<&SbrPrevFrameData>,
) {
    let mut f_frame_error = h_header_data.frame_error_flag as i32;

    if f_frame_error == 0 {
        // To avoid distortions after bad frames, set the error flag if delta coding in time
        // occurs. However, SBR can take a little longer to come up again.
        if h_prev_data.frame_error_flag != 0 {
            if h_sbr_data.domain_vec[0] != 0 {
                f_frame_error = 1;
            }
        } else {
            // Check that the previous stop position and the current start position match.
            // (Could be done in checkFrameInfo(), but the previous frame data is not available
            // there)
            if h_sbr_data.frame_info.borders[0] as i32
                != h_prev_data.stop_pos as i32 - h_header_data.number_time_slots as i32
            {
                // Both the previous as well as the current frame are flagged to be ok, but they
                // do not match! Prefer concealment over delta-time coding between the
                // mismatching frames, or close the gap in time by triggering
                // timeCompensateFirstEnvelope().
                f_frame_error = 1;
            }
        }
    }

    if f_frame_error != 0 {
        // Error is detected
        lean_sbr_concealment(h_header_data, h_sbr_data, h_prev_data);

        // decode the envelope data to linear PCM
        delta_to_linear_pcm_envelope_decoding(h_header_data, h_sbr_data, h_prev_data);
    } else {
        // Do a temporary dummy decoding and check that the envelope values are within limits
        if h_prev_data.frame_error_flag != 0 {
            time_compensate_first_envelope(h_header_data, h_sbr_data, h_prev_data);
            if h_sbr_data.coupling != h_prev_data.coupling {
                // Coupling mode has changed during concealment. The stored energy levels need
                // to be converted.
                for i in 0..(h_header_data.freq_band_data.n_sfb[1] as usize).min(MAX_FREQ_COEFFS) {
                    // Former Level-Channel will be used for both channels
                    if h_prev_data.coupling == COUPLING_BAL {
                        h_prev_data.sfb_nrg_prev[i] = match other_channel {
                            Some(o) => o.sfb_nrg_prev[i],
                            None => SBR_ENERGY_PAN_OFFSET,
                        };
                    }
                    // Former L/R will be combined as the new Level-Channel
                    else if h_sbr_data.coupling == COUPLING_LEVEL && other_channel.is_some() {
                        let o = other_channel.map_or(0, |o| o.sfb_nrg_prev[i]);
                        h_prev_data.sfb_nrg_prev[i] =
                            ((h_prev_data.sfb_nrg_prev[i] as i32 + o as i32) >> 1) as FixpSgl;
                    } else if h_sbr_data.coupling == COUPLING_BAL {
                        h_prev_data.sfb_nrg_prev[i] = SBR_ENERGY_PAN_OFFSET;
                    }
                }
            }
        }
        let temp_sfb_nrg_prev = h_prev_data.sfb_nrg_prev;

        delta_to_linear_pcm_envelope_decoding(h_header_data, h_sbr_data, h_prev_data);

        f_frame_error = check_envelope_data(h_header_data, h_sbr_data, h_prev_data);

        if f_frame_error != 0 {
            h_header_data.frame_error_flag = 1;
            h_prev_data.sfb_nrg_prev = temp_sfb_nrg_prev;
            decode_envelope(h_header_data, h_sbr_data, h_prev_data, other_channel);
            return;
        }
    }

    requantize_envelope_data(h_sbr_data, h_sbr_data.amp_resolution_current_frame);

    h_header_data.frame_error_flag = f_frame_error as u8;
}

/// `checkEnvelopeData()`: verifies that the envelope energies are within the allowed range.
/// Returns 1 if an envelope value was out of range.
fn check_envelope_data(
    h_header_data: &SbrHeaderData,
    h_sbr_data: &SbrFrameData,
    h_prev_data: &mut SbrPrevFrameData,
) -> i32 {
    let mut error_flag = 0;
    let sbr_max_energy: FixpSgl = if h_sbr_data.amp_resolution_current_frame == 1 {
        SBR_MAX_ENERGY
    } else {
        SBR_MAX_ENERGY << 1
    };

    // Range check for current energies
    for &e in h_sbr_data.i_envelope
        [..(h_sbr_data.n_scale_factors.max(0) as usize).min(h_sbr_data.i_envelope.len())]
        .iter()
    {
        if e > sbr_max_energy {
            error_flag = 1;
        }
        if e < 0 {
            error_flag = 1;
        }
    }

    // Range check for previous energies
    for v in h_prev_data.sfb_nrg_prev
        [..(h_header_data.freq_band_data.n_sfb[1] as usize).min(MAX_FREQ_COEFFS)]
        .iter_mut()
    {
        *v = fix_max(*v, 0);
        *v = fix_min(*v, sbr_max_energy);
    }

    error_flag
}

/// `limitNoiseLevels()`: limits the noise levels to the allowed range.
fn limit_noise_levels(h_header_data: &SbrHeaderData, h_sbr_data: &mut SbrFrameData) {
    let n_nfb = h_header_data.freq_band_data.n_nfb as usize;

    // lowerLimit actually refers to the _highest_ noise energy, upperLimit actually refers to the
    // _lowest_ noise energy
    const LOWER_LIMIT: FixpSgl = 0;
    const UPPER_LIMIT: FixpSgl = 35;

    // Range check for current noise levels
    let n = (h_sbr_data.frame_info.n_noise_envelopes as usize * n_nfb)
        .min(h_sbr_data.sbr_noise_floor_level.len());
    for v in h_sbr_data.sbr_noise_floor_level[..n].iter_mut() {
        *v = fix_min(*v, UPPER_LIMIT);
        *v = fix_max(*v, LOWER_LIMIT);
    }
}

/// `timeCompensateFirstEnvelope()`: compensates for the wrong timing that might occur after a
/// frame error.
fn time_compensate_first_envelope(
    h_header_data: &SbrHeaderData,
    h_sbr_data: &mut SbrFrameData,
    h_prev_data: &SbrPrevFrameData,
) {
    let n_sfb = h_header_data.freq_band_data.n_sfb;
    let p_frame_info = &mut h_sbr_data.frame_info;
    let mut estimated_start_pos =
        f_max(0, h_prev_data.stop_pos as i32 - h_header_data.number_time_slots as i32);

    // Original length of first envelope according to bitstream
    let ref_len = p_frame_info.borders[1] as i32 - p_frame_info.borders[0] as i32;
    // Corrected length of first envelope (concealing can make the first envelope longer)
    let mut new_len = p_frame_info.borders[1] as i32 - estimated_start_pos;

    if new_len <= 0 {
        // An envelope length of <= 0 would not work, so we don't use it. May occur if the
        // previous frame was flagged bad due to a mismatch of the old and new frame infos.
        new_len = ref_len;
        estimated_start_pos = p_frame_info.borders[0] as i32;
    }

    let mut delta_exp = fdk_get_num_octaves_div8(new_len, ref_len);

    // Shift by -3 to rescale ld-table, ampRes-1 to enable coarser steps
    let shift = FRACT_BITS - 1 - ENV_EXP_FRACT - 1 + h_sbr_data.amp_resolution_current_frame - 3;
    // shift is 11 or 12 (ampResolutionCurrentFrame is 0 or 1)
    delta_exp = ((delta_exp as i32).wrapping_shr(shift as u32)) as FixpSgl;
    p_frame_info.borders[0] = estimated_start_pos as u8;
    p_frame_info.borders_noise[0] = estimated_start_pos as u8;

    if h_sbr_data.coupling != COUPLING_BAL {
        let n_scalefactors =
            if p_frame_info.freq_res[0] != 0 { n_sfb[1] } else { n_sfb[0] } as usize;

        for v in h_sbr_data.i_envelope[..n_scalefactors.min(MAX_FREQ_COEFFS)].iter_mut() {
            *v = v.wrapping_add(delta_exp);
        }
    }
}

/// `requantizeEnvelopeData()`: converts each envelope value from logarithmic to linear domain
/// (the pseudo float format of mantissa and exponent in one `FIXP_SGL`).
fn requantize_envelope_data(h_sbr_data: &mut SbrFrameData, amp_resolution: i32) {
    let amp_shift = 1 - amp_resolution;

    let n = (h_sbr_data.n_scale_factors.max(0) as usize).min(h_sbr_data.i_envelope.len());
    for v in h_sbr_data.i_envelope[..n].iter_mut() {
        let mut exponent = *v as i32;

        // In case of the high amplitude resolution, 1 bit of the exponent gets lost by the
        // shift. This will be compensated by a mantissa of 0.5*sqrt(2) instead of 0.5 if that
        // bit is 1. (ampShift is 0 or 1)
        let mantissa = if exponent & amp_shift != 0 {
            fl2fxconst_sgl(0.707106781186548f32 as f64)
        } else {
            fl2fxconst_sgl(0.5f32 as f64)
        };
        exponent = exponent.wrapping_shr(amp_shift as u32);

        // Mantissa was set to 0.5 (instead of 1.0, therefore increase exponent by 1). Multiply
        // by L=nChannels=64 by increasing exponent by another 6. => Increase exponent by 7
        exponent += 7 + NRG_EXP_OFFSET;

        // Combine mantissa and exponent and write back the result
        let m = (mantissa as i32 & MASK_M) as FixpSgl;
        let e = (exponent as i16 as i32 & MASK_E) as FixpSgl;
        *v = (m as i32 + e as i32) as FixpSgl;
    }
}

/// `deltaToLinearPcmEnvelopeDecoding()`: builds the new reference energies from the old ones and
/// the delta coded data.
fn delta_to_linear_pcm_envelope_decoding(
    h_header_data: &SbrHeaderData,
    h_sbr_data: &mut SbrFrameData,
    h_prev_data: &mut SbrPrevFrameData,
) {
    let fbd = &h_header_data.freq_band_data;
    let sfb_nrg_prev = &mut h_prev_data.sfb_nrg_prev;
    let mut p = 0usize; // ptr_nrg

    let offset = 2 * fbd.n_sfb[0] as i32 - fbd.n_sfb[1] as i32;

    for i in 0..(h_sbr_data.frame_info.n_envelopes as usize).min(MAX_FREQ_COEFFS) {
        let domain = h_sbr_data.domain_vec.get(i).copied().unwrap_or(0);
        let freq_res = h_sbr_data.frame_info.freq_res.get(i).copied().unwrap_or(0) as i32;

        fdk_assert!((0..=1).contains(&freq_res));

        let no_of_bands = fbd.n_sfb_at(freq_res as usize) as i32;

        fdk_assert!(no_of_bands < 64);

        // The envelope values of all envelopes fit into iEnvelope[] (at most 8 envelopes of at
        // most 56 bands); the port stops at its end.
        if domain == 0 {
            let Some(&v0) = h_sbr_data.i_envelope.get(p) else {
                return;
            };
            map_low_res_energy_val(v0, sfb_nrg_prev, offset, 0, freq_res);
            p += 1;
            for band in 1..no_of_bands {
                if p >= h_sbr_data.i_envelope.len() {
                    return;
                }
                let v = h_sbr_data.i_envelope[p].wrapping_add(h_sbr_data.i_envelope[p - 1]);
                h_sbr_data.i_envelope[p] = v;
                map_low_res_energy_val(v, sfb_nrg_prev, offset, band, freq_res);
                p += 1;
            }
        } else {
            for band in 0..no_of_bands {
                if p >= h_sbr_data.i_envelope.len() {
                    return;
                }
                let prev = sfb_nrg_prev
                    .get(index_low2_high(offset, band, freq_res) as usize)
                    .copied()
                    .unwrap_or(0);
                let v = h_sbr_data.i_envelope[p].wrapping_add(prev);
                h_sbr_data.i_envelope[p] = v;
                map_low_res_energy_val(v, sfb_nrg_prev, offset, band, freq_res);
                p += 1;
            }
        }
    }
}

/// `decodeNoiseFloorlevels()`: builds the new noise levels from the old ones and the delta coded
/// data.
fn decode_noise_floorlevels(
    h_header_data: &SbrHeaderData,
    h_sbr_data: &mut SbrFrameData,
    h_prev_data: &mut SbrPrevFrameData,
) {
    // nNfb <= MAX_NOISE_COEFFS (resetFreqBandTables())
    let n_nfb = (h_header_data.freq_band_data.n_nfb as usize).min(5);
    let n_noise_floor_envelopes = h_sbr_data.frame_info.n_noise_envelopes as usize;
    let nf = &mut h_sbr_data.sbr_noise_floor_level;

    // Decode first noise envelope

    if h_sbr_data.domain_vec_noise[0] == 0 {
        let mut noise_level = nf[0];
        for i in 1..n_nfb {
            noise_level = noise_level.wrapping_add(nf[i]);
            nf[i] = noise_level;
        }
    } else {
        for i in 0..n_nfb {
            nf[i] = nf[i].wrapping_add(h_prev_data.prev_noise_level[i]);
        }
    }

    // If present, decode the second noise envelope. Note: nNoiseFloorEnvelopes can only be 1
    // or 2
    if n_noise_floor_envelopes > 1 {
        if h_sbr_data.domain_vec_noise[1] == 0 {
            let mut noise_level = nf[n_nfb];
            for i in n_nfb + 1..2 * n_nfb {
                noise_level = noise_level.wrapping_add(nf[i]);
                nf[i] = noise_level;
            }
        } else {
            for i in 0..n_nfb {
                nf[i + n_nfb] = nf[i + n_nfb].wrapping_add(nf[i]);
            }
        }
    }

    limit_noise_levels(h_header_data, h_sbr_data);
    let nf = &mut h_sbr_data.sbr_noise_floor_level;

    // Update prevNoiseLevel with the last noise envelope
    for i in 0..n_nfb {
        h_prev_data.prev_noise_level[i] =
            nf.get(i + n_nfb * n_noise_floor_envelopes.wrapping_sub(1)).copied().unwrap_or(0);
    }

    // Requantize the noise floor levels in COUPLING_OFF-mode
    if h_sbr_data.coupling == 0 {
        for v in nf[..(n_noise_floor_envelopes * n_nfb).min(10)].iter_mut() {
            // +1 to compensate for a mantissa of 0.5 instead of 1.0
            let nf_e = 6 - *v as i32 + 1 + NOISE_EXP_OFFSET;

            // mantissa + exponent
            *v = (fl2fxconst_sgl(0.5f32 as f64) as i32 + (nf_e & MASK_E)) as FixpSgl;
        }
    }
}
