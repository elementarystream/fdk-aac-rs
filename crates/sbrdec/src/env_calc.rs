// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libSBRdec/src/env_calc.h, libSBRdec/src/env_calc.cpp.

//! Port of `env_calc.h` and `env_calc.cpp`: the envelope adjustor. It compares the energies of
//! the transposed high band with the reference energies of the bitstream, amplifies or
//! attenuates the high band, adds noise (the random noise state `phaseIndex` never converges and
//! is reproduced exactly) and synthetic sines, and limits the gains.
//!
//! # Frequency band table redirection
//!
//! For PVC frames whose first noise envelope is the trailing noise envelope of the previous
//! frame, `calculateSbrEnvelope()` redirects the header's `freqBandTable[0..2]` pointers to the
//! tables of the previous frame (`prevFreqBandTableLo/Hi`) and restores them when the current
//! noise envelope starts. The noise border of a PVC frame is at most 3, before the end of the
//! envelope loop (16), so the pointers are always restored; the port keeps the redirection local
//! (copies of the tables, which do not change within the call).
//!
//! # Slots
//!
//! The QMF data is a slice of slots (`&mut [&mut [FixpDbl]]`, 64 values each, see
//! [`crate::sbr_dec`]); a `NULL` imaginary buffer (low power mode) is an empty slice.
//!
//! # Variants
//!
//! The `.ii` files differ only in `SATURATE_SHIFT` of `adjustTimeSlot_EldGrid()` (ported per
//! variant in `fdk_aac_rs_fdk::scale`); its amount `scale_diff_low` is limited to -31 from below
//! by the caller and `EXP2SCALE(adj_e) - lb_scale` from above, which stays far below 32.
//!
//! # Undefined behavior
//!
//! Shifts by exponent differences that the C code does not limit (`>>= -sc_change`, `MAXVAL_DBL >>
//! gain_sf[i]`, ...) are register shifts in both builds (amount modulo 32): `wrapping_shr`.
//! Additions that can overflow wrap.

pub mod tables;
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
mod x86;

use crate::env_extr::*;
use crate::hbe::{MAX_NUM_PATCHES_HBE, MAX_STRETCH_HBE};
use crate::lpp_tran::*;
use crate::pvc_dec::{expand_pred_esg, PvcDynamicData, PVC_NTIMESLOT};
use crate::sbr_ram::pod_struct;
use crate::sbr_rom::*;
use crate::sbrdec_freq_sca::shellsort;
use crate::sbrdecoder::{SbrError, SBRDEC_OK, SBRDEC_UNSUPPORTED_CONFIG};
use crate::transcendent::*;
use fdk_aac_rs_fdk::common_fix::*;
use fdk_aac_rs_fdk::fdk_assert;
use fdk_aac_rs_fdk::qmf::QmfScaleFactor;
use tables::*;

/// `MAX_SFB_NRG_HEADROOM`
const MAX_SFB_NRG_HEADROOM: i32 = 1;
/// `MAX_VAL_NRG_HEADROOM`
const MAX_VAL_NRG_HEADROOM: FixpDbl = MAXVAL_DBL >> MAX_SFB_NRG_HEADROOM;
/// `INTER_TES_SF_CHANGE`
const INTER_TES_SF_CHANGE: i32 = 4;
/// `SHIFT_BEFORE_SQUARE`
const SHIFT_BEFORE_SQUARE: i32 = 4;

pod_struct! {
    /// `SBR_CALCULATE_ENVELOPE`
    pub struct SbrCalculateEnvelope {
        /// `filtBuffer`: previous gains (required for smoothing)
        pub filt_buffer: [FixpDbl; MAX_FREQ_COEFFS],
        /// `filtBufferNoise`: previous noise levels (required for smoothing)
        pub filt_buffer_noise: [FixpDbl; MAX_FREQ_COEFFS],
        /// `filtBuffer_e`: exponents of previous gains
        pub filt_buffer_e: [i8; MAX_FREQ_COEFFS],
        /// `filtBufferNoise_e`: common exponent of previous noise levels
        pub filt_buffer_noise_e: i8,
        pub _pad0: [u8; 3],
        /// `startUp`: flag to signal initial conditions in buffers
        pub start_up: i32,
        /// `phaseIndex`: index for randomPase array
        pub phase_index: i32,
        /// `prevTranEnv`: the transient envelope of the previous frame
        pub prev_tran_env: i32,
        /// `harmFlagsPrev`: words with 16 flags each indicating where a sine was added in the
        /// previous frame
        pub harm_flags_prev: [u32; ADD_HARMONICS_FLAGS_SIZE],
        /// `harmIndex`: current phase of synthetic sine
        pub harm_index: u8,
        pub _pad1: [u8; 3],
        /// `sbrPatchingMode`: current patching mode
        pub sbr_patching_mode: i32,
        /// `prevSbrNoiseFloorLevel`
        pub prev_sbr_noise_floor_level: [FixpSgl; MAX_NOISE_COEFFS],
        /// `prevNNfb`
        pub prev_n_nfb: u8,
        /// `prevNSfb`
        pub prev_n_sfb: [u8; 2],
        /// `prevLoSubband`
        pub prev_lo_subband: u8,
        /// `prevHiSubband`
        pub prev_hi_subband: u8,
        /// `prev_ov_highSubband`
        pub prev_ov_high_subband: u8,
        /// `prevFreqBandTable[2]` (placeholder, never used by the reference)
        pub _prev_freq_band_table: [usize; 2],
        /// `prevFreqBandTableLo`
        pub prev_freq_band_table_lo: [u8; MAX_FREQ_COEFFS / 2 + 1],
        /// `prevFreqBandTableHi`
        pub prev_freq_band_table_hi: [u8; MAX_FREQ_COEFFS + 1],
        /// `prevFreqBandTableNoise`
        pub prev_freq_band_table_noise: [u8; MAX_NOISE_COEFFS + 1],
        /// `sinusoidal_positionPrev`
        pub sinusoidal_position_prev: i8,
        pub _pad2: [u8; 3],
        /// `harmFlagsPrevActive`
        pub harm_flags_prev_active: [u32; ADD_HARMONICS_FLAGS_SIZE],
    }
}

pod_struct! {
    /// `ENV_CALC_NRGS`: the energies of one envelope (scratch memory, cleared for every
    /// envelope). The layout is the one of the reference (the differential tests pass it as
    /// memory).
    pub struct EnvCalcNrgs {
        /// `nrgRef`
        pub nrg_ref: [FixpDbl; MAX_FREQ_COEFFS],
        /// `nrgEst`
        pub nrg_est: [FixpDbl; MAX_FREQ_COEFFS],
        /// `nrgGain`
        pub nrg_gain: [FixpDbl; MAX_FREQ_COEFFS],
        /// `noiseLevel`
        pub noise_level: [FixpDbl; MAX_FREQ_COEFFS],
        /// `nrgSine`
        pub nrg_sine: [FixpDbl; MAX_FREQ_COEFFS],
        /// `nrgRef_e`
        pub nrg_ref_e: [i8; MAX_FREQ_COEFFS],
        /// `nrgEst_e`
        pub nrg_est_e: [i8; MAX_FREQ_COEFFS],
        /// `nrgGain_e`
        pub nrg_gain_e: [i8; MAX_FREQ_COEFFS],
        /// `noiseLevel_e`
        pub noise_level_e: [i8; MAX_FREQ_COEFFS],
        /// `nrgSine_e`
        pub nrg_sine_e: [i8; MAX_FREQ_COEFFS],
        /// `exponent`: yet another exponent `[0]`: for ts < no_cols; `[1]`: for ts >= no_cols
        pub exponent: [i8; 2],
        pub _pad0: [u8; 2],
    }
}

/// `mapSineFlags()`: maps the sine flags of the bitstream (one per SBR band) to QMF bands and
/// to the envelope where the sine shall start (`sineMapped`, 32 means "no sine"), and updates
/// the flags of the previous frame.
pub fn map_sine_flags(
    freq_band_table: &[u8],
    n_sfb: i32,
    add_harmonics: &[u32; ADD_HARMONICS_FLAGS_SIZE],
    harm_flags_prev: &mut [u32; ADD_HARMONICS_FLAGS_SIZE],
    harm_flags_prev_active: &mut [u32; ADD_HARMONICS_FLAGS_SIZE],
    tran_env: i32,
    sine_mapped: &mut [i8; MAX_FREQ_COEFFS],
) {
    let mut bitcount = 31i32;
    let mut harm_flags_qmf_bands = [0u32; ADD_HARMONICS_FLAGS_SIZE];
    let mut cur_flags = 0usize;
    let fbt = |i: usize| freq_band_table.get(i).copied().unwrap_or(0) as i32;

    // Reset the output vector first
    sine_mapped.fill(32); // 32 means 'no sine'
    harm_flags_prev_active.fill(0);
    for i in 0..n_sfb.max(0) as usize {
        // mask to extract addHarmonics flag of current Sfb
        let mask_sfb = 1u32 << bitcount;

        // addHarmonics has 2 words; nSfb <= 56
        if add_harmonics.get(cur_flags).copied().unwrap_or(0) & mask_sfb != 0 {
            // There is a sine in this band
            let lsb = fbt(0); // start of sbr range
                              // qmf band to which sine should be added
            let qmf_band = (fbt(i) + fbt(i + 1)) >> 1;
            let qmf_band_div32 = (qmf_band >> 5) as usize & 1;
            // mask to extract harmonic flag from prevFlags
            let mask_qmf_band = 1u32 << (qmf_band & 31);

            // mapping of sfb with sine to a certain qmf band -> for harmFlagsPrev
            harm_flags_qmf_bands[qmf_band_div32] |= mask_qmf_band;

            // If there was a sine in the last frame, let it continue from the first envelope on
            // else start at the transient position. Indexing of sineMapped starts relative to
            // lsb.
            let v = if harm_flags_prev[qmf_band_div32] & mask_qmf_band != 0 {
                0
            } else {
                tran_env as i8
            };
            // `qmfBand - lsb` outside of sineMapped[] has no defined result in the reference;
            // the port skips the write (deviation) and uses the value.
            if let Some(s) = sine_mapped.get_mut((qmf_band - lsb) as usize) {
                *s = v;
            }
            if (v as i32) < PVC_NTIMESLOT as i32 {
                harm_flags_prev_active[qmf_band_div32] |= mask_qmf_band;
            }
        }

        if bitcount == 0 {
            bitcount = 31;
            cur_flags += 1;
        } else {
            bitcount -= 1;
        }
    }
    *harm_flags_prev = harm_flags_qmf_bands;
}

/// `mapSineFlagsPvc()`: restores the sine mapping of the previous frame for the time slots
/// where the SBR framing is trailing the PVC framing.
#[allow(clippy::too_many_arguments)]
pub fn map_sine_flags_pvc(
    freq_band_table: &[u8],
    n_sfb: i32,
    harm_flags_prev: &[u32; ADD_HARMONICS_FLAGS_SIZE],
    harm_flags_prev_active: &[u32; ADD_HARMONICS_FLAGS_SIZE],
    sine_mapped: &mut [i8; MAX_FREQ_COEFFS],
    sinusoidal_pos: i32,
    sinusoidal_pos_prev: &mut i8,
    trailing_sbr_frame: i32,
) {
    // Reset the output vector first
    sine_mapped.fill(32); // 32 means 'no sine'

    if trailing_sbr_frame != 0 {
        // restore sineMapped[] of previous frame
        let lsb = freq_band_table.first().copied().unwrap_or(0) as i32;
        let usb = freq_band_table.get(n_sfb.max(0) as usize).copied().unwrap_or(0) as i32;
        // (the port skips the bands beyond MAX_FREQ_COEFFS, for which the reference has no
        // defined result)
        for i in lsb..usb.min(lsb + MAX_FREQ_COEFFS as i32) {
            let qmf_band_div32 = (i >> 5) as usize & 1;
            // mask to extract harmonic flag from prevFlags
            let mask_qmf_band = 1u32 << (i & 31);

            // Two cases need to be distinguished ...
            if harm_flags_prev_active[qmf_band_div32] & mask_qmf_band != 0 {
                // the sine mapping already started last PVC frame -> seamlessly continue
                sine_mapped[(i - lsb) as usize] = 0;
            } else if harm_flags_prev[qmf_band_div32] & mask_qmf_band != 0 {
                // sinusoidalPos of prev PVC frame was >= PVC_NTIMESLOT -> sine starts in this
                // frame; we are 16 sbr time slots ahead of last frame now
                sine_mapped[(i - lsb) as usize] =
                    (*sinusoidal_pos_prev as i32 - PVC_NTIMESLOT as i32) as i8;
            }
        }
    }
    *sinusoidal_pos_prev = sinusoidal_pos as i8;
}

/// `aliasingReduction()`: reduces the gain-adjustment induced aliasing of the real valued
/// filter bank. `degree_alias` starts at `lowSubband`.
pub fn aliasing_reduction(
    degree_alias: &[FixpDbl],
    nrgs: &mut EnvCalcNrgs,
    use_alias_reduction: &[u8],
    no_subbands: i32,
) {
    let mut grouping = false;
    let mut index = 0usize;
    let mut group_vector = [0i32; MAX_FREQ_COEFFS];
    let da = |k: i32| degree_alias.get(k as usize).copied().unwrap_or(0);

    // Calculate grouping
    let mut k = 0i32;
    while k < no_subbands - 1 {
        if da(k + 1) != 0 && use_alias_reduction[k as usize] != 0 {
            if !grouping {
                group_vector[index] = k;
                index += 1;
                grouping = true;
            } else if group_vector[index - 1] + 3 == k {
                group_vector[index] = k + 1;
                index += 1;
                grouping = false;
            }
        } else if grouping {
            if use_alias_reduction[k as usize] != 0 {
                group_vector[index] = k + 1;
            } else {
                group_vector[index] = k;
            }
            index += 1;
            grouping = false;
        }
        k += 1;
    }

    if grouping {
        group_vector[index] = no_subbands;
        index += 1;
    }
    let no_groups = index >> 1;

    // Calculate new gain
    for group in 0..no_groups {
        let mut nrg_orig: FixpDbl = 0; // Original signal energy in current group of bands
        let mut nrg_orig_e: i8 = 0;
        let mut nrg_amp: FixpDbl = 0; // Amplified signal energy in group (using current gains)
        let mut nrg_amp_e: i8 = 0;
        let mut nrg_mod: FixpDbl = 0; // Signal energy in group when applying modified gains
        let mut nrg_mod_e: i8 = 0;

        let start_group = group_vector[2 * group];
        let stop_group = group_vector[2 * group + 1];

        // Calculate total energy in group before and after amplification with current gains:
        for k in start_group..stop_group {
            let k = k as usize;
            // Get original band energy
            let mut tmp = nrgs.nrg_est[k];
            let mut tmp_e = nrgs.nrg_est_e[k];

            (nrg_orig, nrg_orig_e) = fdk_add_mant_exp(tmp, tmp_e, nrg_orig, nrg_orig_e);

            // Multiply band energy with current gain
            tmp = f_mult(tmp, nrgs.nrg_gain[k]);
            tmp_e = tmp_e.wrapping_add(nrgs.nrg_gain_e[k]);

            (nrg_amp, nrg_amp_e) = fdk_add_mant_exp(tmp, tmp_e, nrg_amp, nrg_amp_e);
        }

        // Calculate total energy gain in group
        let (group_gain, group_gain_e) =
            fdk_divide_mant_exp(nrg_amp, nrg_amp_e, nrg_orig, nrg_orig_e);

        for k in start_group..stop_group {
            let ku = k as usize;
            let mut alpha = da(k);
            if k < no_subbands - 1 && da(k + 1) > alpha {
                alpha = da(k + 1);
            }

            // Modify gain depending on the degree of aliasing
            (nrgs.nrg_gain[ku], nrgs.nrg_gain_e[ku]) = fdk_add_mant_exp(
                f_mult(alpha, group_gain),
                group_gain_e,
                f_mult(MAXVAL_DBL.wrapping_sub(alpha), nrgs.nrg_gain[ku]),
                nrgs.nrg_gain_e[ku],
            );

            // Apply modified gain to original energy
            let tmp = f_mult(nrgs.nrg_gain[ku], nrgs.nrg_est[ku]);
            let tmp_e = nrgs.nrg_gain_e[ku].wrapping_add(nrgs.nrg_est_e[ku]);

            // Accumulate energy with modified gains applied
            (nrg_mod, nrg_mod_e) = fdk_add_mant_exp(tmp, tmp_e, nrg_mod, nrg_mod_e);
        }

        // Calculate compensation factor to retain the energy of the amplified signal
        let (compensation, compensation_e) =
            fdk_divide_mant_exp(nrg_amp, nrg_amp_e, nrg_mod, nrg_mod_e);

        // Apply compensation factor to all gains of the group
        for k in start_group..stop_group {
            let k = k as usize;
            nrgs.nrg_gain[k] = f_mult(nrgs.nrg_gain[k], compensation);
            nrgs.nrg_gain_e[k] = nrgs.nrg_gain_e[k].wrapping_add(compensation_e);
        }
    }
}

/// `apply_inter_tes()`: the inter subband-sample temporal envelope shaping of USAC.
#[allow(clippy::too_many_arguments)]
pub fn apply_inter_tes(
    qmf_real: &mut [&mut [FixpDbl]],
    qmf_imag: &mut [&mut [FixpDbl]],
    sbr_scale_factor: &QmfScaleFactor,
    exp: &[i8; 2],
    rate: i32,
    start_pos: i32,
    stop_pos: i32,
    low_subband: i32,
    nb_subband: i32,
    gamma_idx: u8,
) {
    let high_subband = low_subband + nb_subband;
    const N: usize = MAX_COLS + MAX_OV_COLS;
    // ITES_TEMP (scratch): every value used is written first
    let mut subsample_power_low = [0 as FixpDbl; N];
    let mut subsample_power_high = [0 as FixpDbl; N];
    let mut gain = [0 as FixpDbl; N];
    let mut subsample_power_low_sf = [0i8; N];
    let mut subsample_power_high_sf = [0i8; N];
    let mut gain_sf = [0i32; N];
    let mut total_power_high: FixpDbl = 0;
    let mut total_power_low: FixpDbl = 0;

    // gamma[gamma_idx] = {0.0f, 1.0f, 2.0f, 4.0f}
    let gamma_sf = gamma_idx as i32 - 1; // perhaps +1 to save one bit? (0.99999f vs 1.f)

    let nb_subsample = (stop_pos - start_pos).clamp(0, N as i32);
    let lo = low_subband.clamp(0, 64) as usize;
    let hi = high_subband.clamp(lo as i32, 64) as usize;

    if gamma_idx > 0 {
        let pre_shift2 = 32 - f_normz(nb_subsample);
        let mut total_power_low_sf = 1 - DFRACT_BITS;
        let mut total_power_high_sf = 1 - DFRACT_BITS;

        for i in 0..nb_subsample as usize {
            let mut buffer_real = [0 as FixpDbl; N];
            let mut buffer_imag = [0 as FixpDbl; N];
            let mut max_val: FixpDbl = 0;

            let ts = start_pos + i as i32;
            let slot = ts as usize;

            let mut low_sf = if ts < 3 * rate {
                sbr_scale_factor.ov_lb_scale
            } else {
                sbr_scale_factor.lb_scale
            };
            low_sf = 15 - low_sf;

            for j in 0..lo {
                buffer_imag[j] = qmf_imag[slot][j];
                max_val |= buffer_imag[j] ^ (buffer_imag[j] >> (DFRACT_BITS - 1));
                buffer_real[j] = qmf_real[slot][j];
                max_val |= buffer_real[j] ^ (buffer_real[j] >> (DFRACT_BITS - 1));
            }

            subsample_power_low[i] = 0;
            subsample_power_low_sf[i] = 0;

            if max_val != 0 {
                // multiply first, then shift for safe summation
                let mut pre_shift = 1 - cnt_leading_zeros(max_val);
                let post_shift = 32 - f_normz(low_subband);

                // reduce preShift because otherwise we risk to square -1.f
                if pre_shift != 0 {
                    pre_shift += 1;
                }

                subsample_power_low_sf[i] =
                    (subsample_power_low_sf[i] as i32 + (low_sf + pre_shift) * 2 + post_shift + 1)
                        as i8;

                scale_values(&mut buffer_real[..lo], -pre_shift);
                scale_values(&mut buffer_imag[..lo], -pre_shift);
                for j in 0..lo {
                    let addme = f_pow2_div2(buffer_real[j]);
                    subsample_power_low[i] =
                        subsample_power_low[i].wrapping_add(addme.wrapping_shr(post_shift as u32));
                    let addme = f_pow2_div2(buffer_imag[j]);
                    subsample_power_low[i] =
                        subsample_power_low[i].wrapping_add(addme.wrapping_shr(post_shift as u32));
                }
            }

            // now get high

            max_val = 0;

            let high_sf = exp[if ts < 16 * rate { 0 } else { 1 }] as i32;

            for j in lo..hi {
                buffer_imag[j] = qmf_imag[slot][j];
                max_val |= buffer_imag[j] ^ (buffer_imag[j] >> (DFRACT_BITS - 1));
                buffer_real[j] = qmf_real[slot][j];
                max_val |= buffer_real[j] ^ (buffer_real[j] >> (DFRACT_BITS - 1));
            }

            subsample_power_high[i] = 0;
            subsample_power_high_sf[i] = 0;

            if max_val != 0 {
                let mut pre_shift = 1 - cnt_leading_zeros(max_val);
                // reduce preShift because otherwise we risk to square -1.f
                if pre_shift != 0 {
                    pre_shift += 1;
                }

                let post_shift = 32 - f_normz(high_subband - low_subband);
                subsample_power_high_sf[i] = (subsample_power_high_sf[i] as i32
                    + (high_sf + pre_shift) * 2
                    + post_shift
                    + 1) as i8;

                scale_values(&mut buffer_real[lo..hi], -pre_shift);
                scale_values(&mut buffer_imag[lo..hi], -pre_shift);
                for j in lo..hi {
                    subsample_power_high[i] = subsample_power_high[i]
                        .wrapping_add(f_pow2_div2(buffer_real[j]).wrapping_shr(post_shift as u32));
                    subsample_power_high[i] = subsample_power_high[i]
                        .wrapping_add(f_pow2_div2(buffer_imag[j]).wrapping_shr(post_shift as u32));
                }
            }

            // sum all together
            let mut new_summand = subsample_power_low[i];
            let mut new_summand_sf = subsample_power_low_sf[i] as i32;

            // make sure the current sum, and the new summand have the same SF
            if new_summand_sf > total_power_low_sf {
                let diff = f_min(DFRACT_BITS - 1, new_summand_sf - total_power_low_sf);
                total_power_low >>= diff;
                total_power_low_sf = new_summand_sf;
            } else if new_summand_sf < total_power_low_sf {
                new_summand >>= f_min(DFRACT_BITS - 1, total_power_low_sf - new_summand_sf);
            }

            total_power_low =
                total_power_low.wrapping_add(new_summand.wrapping_shr(pre_shift2 as u32));

            new_summand = subsample_power_high[i];
            new_summand_sf = subsample_power_high_sf[i] as i32;
            if new_summand_sf > total_power_high_sf {
                total_power_high >>= f_min(DFRACT_BITS - 1, new_summand_sf - total_power_high_sf);
                total_power_high_sf = new_summand_sf;
            } else if new_summand_sf < total_power_high_sf {
                new_summand >>= f_min(DFRACT_BITS - 1, total_power_high_sf - new_summand_sf);
            }

            total_power_high =
                total_power_high.wrapping_add(new_summand.wrapping_shr(pre_shift2 as u32));
        }

        total_power_low_sf += pre_shift2;
        total_power_high_sf += pre_shift2;

        // gain[i] = e_LOW[i]
        for i in 0..nb_subsample as usize {
            let mut sf2 = 0;
            let mult = f_mult_norm(subsample_power_low[i], nb_subsample, &mut sf2);
            let mult_sf = subsample_power_low_sf[i] as i32 + DFRACT_BITS - 1 + sf2;

            if total_power_low != 0 {
                gain[i] = f_div_norm_fast(mult, total_power_low, &mut sf2);
                gain_sf[i] = mult_sf - total_power_low_sf + sf2;
                gain[i] = sqrt_fixp_lookup_e(gain[i], &mut gain_sf[i]);
                if gain_sf[i] < 0 {
                    gain[i] >>= f_min(DFRACT_BITS - 1, -gain_sf[i]);
                    gain_sf[i] = 0;
                }
            } else if mult == 0 {
                gain[i] = 0;
                gain_sf[i] = 0;
            } else {
                gain[i] = MAXVAL_DBL;
                gain_sf[i] = 0;
            }
        }

        let mut total_power_high_after: FixpDbl = 0;
        let mut total_power_high_after_sf = 1 - DFRACT_BITS;

        // gain[i] = g_inter[i]
        for i in 0..nb_subsample as usize {
            // calculate: gain[i] = 1.0f + gamma * (gain[i] - 1.0f);
            // to substract this from gain[i]; gain_sf[i] is >= 0 here and can be 32 or more
            // (register shift, amount modulo 32)
            let one = MAXVAL_DBL.wrapping_shr(gain_sf[i] as u32);

            // gamma is actually always 1 according to the table, so skip the fMultDiv2
            let mult = gain[i].wrapping_sub(one) >> 1;
            let mult_sf = gain_sf[i] + gamma_sf;

            let one = fl2fxconst_dbl(0.5f32 as f64).wrapping_shr(mult_sf as u32);
            gain[i] = one.wrapping_add(mult);
            gain_sf[i] += gamma_sf + 1; // +1 because of fMultDiv2()

            // set gain to at least 0.2f
            // limit and calculate gain[i]^2 too
            let gain_pow2: FixpDbl;
            let gain_pow2_sf: i32;

            if f_is_less_than(gain[i], gain_sf[i], fl2fxconst_dbl(0.2f32 as f64), 0) != 0 {
                gain[i] = fl2fxconst_dbl(0.8f32 as f64);
                gain_sf[i] = -2;
                gain_pow2 = fl2fxconst_dbl(0.64f32 as f64);
                gain_pow2_sf = -4;
            } else {
                // this upscaling seems quite important
                let r = count_leading_bits(gain[i]);
                gain[i] <<= r;
                gain_sf[i] -= r;

                gain_pow2 = f_pow2(gain[i]);
                gain_pow2_sf = gain_sf[i] << 1;
            }

            let mut room = 0;
            subsample_power_high[i] = f_mult_norm(subsample_power_high[i], gain_pow2, &mut room);
            subsample_power_high_sf[i] =
                (subsample_power_high_sf[i] as i32 + gain_pow2_sf + room) as i8;

            let new_summand_sf = subsample_power_high_sf[i] as i32; // + gain_pow2_sf;
            if new_summand_sf > total_power_high_after_sf {
                total_power_high_after >>=
                    f_min(DFRACT_BITS - 1, new_summand_sf - total_power_high_after_sf);
                total_power_high_after_sf = new_summand_sf;
            } else if new_summand_sf < total_power_high_after_sf {
                subsample_power_high[i] >>=
                    f_min(DFRACT_BITS - 1, total_power_high_after_sf - new_summand_sf);
            }
            total_power_high_after = total_power_high_after
                .wrapping_add(subsample_power_high[i].wrapping_shr(pre_shift2 as u32));
        }

        total_power_high_after_sf += pre_shift2;

        let mut sf2 = 0;
        let mut gain_adj_2 = fl2fx_dbl(0.5f32);
        let mut gain_adj_2_sf = 1;

        if total_power_high != 0 && total_power_high_after != 0 {
            gain_adj_2 = f_div_norm_fast(total_power_high, total_power_high_after, &mut sf2);
            gain_adj_2_sf = total_power_high_sf - total_power_high_after_sf + sf2;
        }

        let gain_adj = sqrt_fixp_lookup_e(gain_adj_2, &mut gain_adj_2_sf);
        let gain_adj_sf = gain_adj_2_sf;

        for i in 0..nb_subsample as usize {
            let gain_e = f_max(
                f_min(gain_sf[i] + gain_adj_sf - INTER_TES_SF_CHANGE, DFRACT_BITS - 1),
                -(DFRACT_BITS - 1),
            );
            let mut gain_final = f_mult(gain[i], gain_adj);
            gain_final = scale_value_saturate(gain_final, gain_e);

            let slot = (start_pos + i as i32) as usize;
            for j in lo..hi {
                qmf_real[slot][j] = f_mult(qmf_real[slot][j], gain_final);
                qmf_imag[slot][j] = f_mult(qmf_imag[slot][j], gain_final);
            }
        }
    } else {
        // gamma_idx == 0
        // Inter-TES is not active. Still perform the scale change to have a consistent scaling
        // for all envelopes of this frame.
        for i in 0..nb_subsample as usize {
            let slot = (start_pos + i as i32) as usize;
            for j in lo..hi {
                qmf_real[slot][j] >>= INTER_TES_SF_CHANGE;
                qmf_imag[slot][j] >>= INTER_TES_SF_CHANGE;
            }
        }
    }
}

/// The source of the noise levels of `calculateSbrEnvelope()` (`noiseLevels` points to the
/// frame data or, for the trailing noise envelope of a PVC frame, to the previous noise levels).
#[derive(Clone, Copy, PartialEq, Eq)]
enum NoiseSrc {
    Frame,
    Prev,
}

/// `calculateSbrEnvelope()`: applies the spectral envelope to the subband samples of one frame
/// (see the C documentation for the mantissa/exponent formats).
#[allow(clippy::too_many_arguments)]
pub fn calculate_sbr_envelope(
    sbr_scale_factor: &mut QmfScaleFactor,
    h_sbr_cal_env: &mut SbrCalculateEnvelope,
    h_header_data: &SbrHeaderData,
    h_frame_data: &SbrFrameData,
    p_pvc_dynamic_data: &PvcDynamicData,
    analys_buffer_real: &mut [&mut [FixpDbl]],
    analys_buffer_imag: &mut [&mut [FixpDbl]],
    use_lp: bool,
    degree_alias: &mut [FixpDbl],
    flags: u32,
    frame_error_flag: i32,
) {
    let mut env_noise: i32 = 0;
    let borders = &h_frame_data.frame_info.borders;
    let borders_pvc = &h_frame_data.frame_info.pvc_borders;
    let pvc_mode = p_pvc_dynamic_data.pvc_mode as i32;
    let first_start = (if pvc_mode > 0 { borders_pvc[0] } else { borders[0] }) as i32
        * h_header_data.time_step as i32;
    let h_freq = &h_header_data.freq_band_data;

    // noiseLevels: the source and the offset in it
    let mut noise_src = NoiseSrc::Frame;
    let mut noise_off = 0usize;
    let prev_noise_copy = h_sbr_cal_env.prev_sbr_noise_floor_level;
    let noise_at = |src: NoiseSrc, i: usize| -> FixpSgl {
        match src {
            NoiseSrc::Frame => h_frame_data.sbr_noise_floor_level.get(i).copied().unwrap_or(0),
            NoiseSrc::Prev => prev_noise_copy.get(i).copied().unwrap_or(0),
        }
    };

    // pFreqBandTable[0..2] and pFreqBandTableNoise (local copies, see the module documentation)
    let mut tab_lo: [u8; MAX_FREQ_COEFFS / 2 + 1] = h_freq.freq_band_table_lo;
    let mut tab_hi: [u8; MAX_FREQ_COEFFS + 1] = h_freq.freq_band_table_hi;
    let mut tab_noise: [u8; MAX_NOISE_COEFFS + 1] = h_freq.freq_band_table_noise;

    let mut low_subband = h_freq.low_subband as i32;
    let mut high_subband = h_freq.high_subband as i32;
    let mut no_subbands = high_subband - low_subband;

    // old high subband before headerchange we asume no headerchange here
    let mut ov_high_subband = h_freq.high_subband as i32;

    let mut no_noise_bands = h_freq.n_nfb as i32;
    let mut no_sub_frame_bands: [u8; 2] = h_freq.n_sfb;
    let no_cols = h_header_data.number_time_slots as i32 * h_header_data.time_step as i32;

    let mut sine_mapped = [0i8; MAX_FREQ_COEFFS];
    let ov_adj_e = 15i32.wrapping_sub(sbr_scale_factor.ov_hb_scale) as i8; // SCALE2EXP(ov_hb_scale)
    let mut adj_e: i8 = 0;
    let output_e: i8;
    let mut final_e: i8 = 0;
    // inter-TES is active in one or more envelopes of the current SBR frame
    let i_tes_enable = h_frame_data.i_tes_active as i32;
    let i_tes_scale_change = if i_tes_enable != 0 { INTER_TES_SF_CHANGE } else { 0 };
    let max_gain_limit_e: i8 =
        if frame_error_flag != 0 { MAX_GAIN_CONCEAL_EXP as i8 } else { MAX_GAIN_EXP as i8 };

    let mut smooth_length: u8 = 0;

    let mut p_ienv = 0usize;

    // C_ALLOC_SCRATCH_START(useAliasReduction, UCHAR, 64): written before it is read
    let mut use_alias_reduction = [0u8; 64];

    // if values differ we had a headerchange; if old highband is bigger then new one we need to
    // patch overlap-highband-scaling for this frame (see use of ov_highSubband) as overlap
    // contains higher frequency components which would get lost
    if h_freq.high_subband < h_freq.ov_high_subband {
        ov_high_subband = h_freq.ov_high_subband as i32;
    }

    if pvc_mode > 0 {
        if h_frame_data.frame_info.borders_noise[0] > borders_pvc[0] {
            // noise envelope of previous frame is trailing into current PVC frame
            env_noise = -1;
            noise_src = NoiseSrc::Prev;
            noise_off = 0;
            no_noise_bands = h_sbr_cal_env.prev_n_nfb as i32;
            no_sub_frame_bands = h_sbr_cal_env.prev_n_sfb;
            low_subband = h_sbr_cal_env.prev_lo_subband as i32;
            high_subband = h_sbr_cal_env.prev_hi_subband as i32;

            no_subbands = high_subband - low_subband;
            ov_high_subband = high_subband;
            if high_subband < h_sbr_cal_env.prev_ov_high_subband as i32 {
                ov_high_subband = h_sbr_cal_env.prev_ov_high_subband as i32;
            }

            tab_lo = h_sbr_cal_env.prev_freq_band_table_lo;
            tab_hi = h_sbr_cal_env.prev_freq_band_table_hi;
            tab_noise = h_sbr_cal_env.prev_freq_band_table_noise;
        }

        map_sine_flags_pvc(
            &tab_hi,
            no_sub_frame_bands[1] as i32,
            &h_sbr_cal_env.harm_flags_prev,
            &h_sbr_cal_env.harm_flags_prev_active,
            &mut sine_mapped,
            h_frame_data.sinusoidal_position as i32,
            &mut h_sbr_cal_env.sinusoidal_position_prev,
            if borders[0] > borders_pvc[0] { 1 } else { 0 },
        );
    } else {
        // Extract sine flags for all QMF bands
        map_sine_flags(
            &tab_hi,
            no_sub_frame_bands[1] as i32,
            &h_frame_data.add_harmonics,
            &mut h_sbr_cal_env.harm_flags_prev,
            &mut h_sbr_cal_env.harm_flags_prev_active,
            h_frame_data.frame_info.tran_env as i32,
            &mut sine_mapped,
        );
    }

    // Scan for maximum in bufferd noise levels. This is needed in case that we had strong noise
    // in the previous frame which is smoothed into the current frame. The resulting exponent is
    // used as start value for the maximum search in reference energies
    if !use_lp {
        let n = (no_subbands.max(0) as usize).min(MAX_FREQ_COEFFS);
        adj_e = (h_sbr_cal_env.filt_buffer_noise_e as i32
            - get_scalefactor(&h_sbr_cal_env.filt_buffer_noise[..n])
            + MAX_SFB_NRG_HEADROOM) as i8;
    }

    // Scan for maximum reference energy to be able to select appropriate values for adj_e and
    // final_e.
    if pvc_mode > 0 {
        let mut max_sfb_nrg_e = p_pvc_dynamic_data.pred_esg_exp_max;

        // Energy -> magnitude (sqrt halfens exponent)
        max_sfb_nrg_e = (max_sfb_nrg_e + 1) >> 1; // +1 to go safe (round to next higher int)

        // Some safety margin is needed for 2 reasons:
        // - The signal energy is not equally spread over all subband samples in a specific sfb
        //   of an envelope (Nrg could be too high by a factor of envWidth * sfbWidth)
        // - Smoothing can smear high gains of the previous envelope into the current
        max_sfb_nrg_e += 6 + MAX_SFB_NRG_HEADROOM;

        adj_e = max_sfb_nrg_e as i8;
        // final_e should not exist for PVC fixfix framing
    } else {
        for i in 0..h_frame_data.frame_info.n_envelopes as usize {
            let mut max_sfb_nrg_e = -FRACT_BITS + NRG_EXP_OFFSET; // start value for maximum search

            // Fetch frequency resolution for current envelope:
            let fr = h_frame_data.frame_info.freq_res.get(i).copied().unwrap_or(0) as usize;
            let mut j = h_freq.n_sfb_at(fr) as i32;
            if fr < 2 {
                j = no_sub_frame_bands[fr] as i32;
            }
            while j != 0 {
                let v = h_frame_data.i_envelope.get(p_ienv).copied().unwrap_or(0) as i32;
                p_ienv += 1;
                max_sfb_nrg_e = fix_max(max_sfb_nrg_e, v & MASK_E);
                j -= 1;
            }
            max_sfb_nrg_e -= NRG_EXP_OFFSET;

            // Energy -> magnitude (sqrt halfens exponent)
            max_sfb_nrg_e = (max_sfb_nrg_e + 1) >> 1; // +1 to go safe (round to next higher int)

            // Some safety margin is needed (see above)
            max_sfb_nrg_e += 6 + MAX_SFB_NRG_HEADROOM;

            if (borders[i] as i32) < h_header_data.number_time_slots as i32 {
                // This envelope affects timeslots that belong to the output frame
                adj_e = f_max(max_sfb_nrg_e, adj_e as i32) as i8;
            }

            if (borders[i + 1] as i32) > h_header_data.number_time_slots as i32 {
                // This envelope affects timeslots after the output frame
                final_e = f_max(max_sfb_nrg_e, final_e as i32) as i8;
            }
        }
    }

    // Calculate adjustment factors and apply them for every envelope.
    p_ienv = 0;

    let (mut i, i_stop) = if pvc_mode > 0 {
        // iterate over SBR time slots starting with bordersPvc[i]
        // usually 0; can be >0 if switching from legacy SBR to PVC
        fdk_assert!(
            borders_pvc[h_frame_data.frame_info.n_envelopes as usize] as usize == PVC_NTIMESLOT
        );
        (borders_pvc[0] as i32, PVC_NTIMESLOT as i32)
    } else {
        // iterate over SBR envelopes starting with 0
        (0, h_frame_data.frame_info.n_envelopes as i32)
    };
    while i < i_stop {
        let no_noise_flag: i32;
        let mut noise_e: i8;
        let input_e: i8 = 15i32.wrapping_sub(sbr_scale_factor.hb_scale) as i8; // SCALE2EXP(hb_scale)

        // Helper variables.
        let start_pos: i32;
        let stop_pos: i32;
        let freq_res: usize;
        if pvc_mode > 0 {
            // Start-position in time (subband sample) for current envelope.
            start_pos = h_header_data.time_step as i32 * i;
            // Stop-position in time (subband sample) for current envelope.
            stop_pos = h_header_data.time_step as i32 * (i + 1);
            // Frequency resolution for current envelope.
            freq_res = h_frame_data.frame_info.freq_res[0] as usize;
            fdk_assert!(
                freq_res as u8
                    == h_frame_data.frame_info.freq_res[(h_frame_data.frame_info.n_envelopes
                        as usize)
                        .wrapping_sub(1)
                        .min(MAX_ENVELOPES - 1)]
            );
        } else {
            let iu = i as usize;
            start_pos = h_header_data.time_step as i32 * borders[iu] as i32;
            stop_pos = h_header_data.time_step as i32 * borders[iu + 1] as i32;
            freq_res = h_frame_data.frame_info.freq_res[iu] as usize;
        }
        // freqRes is 0 or 1 (one bit)
        let freq_res = freq_res & 1;

        // Always fully initialize the temporary energy table. This prevents negative energies
        // and extreme gain factors in cases where the number of limiter bands exceeds the number
        // of subbands. The latter can be caused by undetected bit errors and is tested by some
        // streams from the certification set.
        let mut nrgs = EnvCalcNrgs::default();

        if pvc_mode > 0 {
            // get predicted energy values from PVC module
            expand_pred_esg(
                p_pvc_dynamic_data,
                i,
                MAX_FREQ_COEFFS as i32,
                &mut nrgs.nrg_ref,
                &mut nrgs.nrg_ref_e,
            );

            if i == borders[0] as i32 {
                map_sine_flags(
                    &tab_hi,
                    no_sub_frame_bands[1] as i32,
                    &h_frame_data.add_harmonics,
                    &mut h_sbr_cal_env.harm_flags_prev,
                    &mut h_sbr_cal_env.harm_flags_prev_active,
                    h_frame_data.sinusoidal_position as i32,
                    &mut sine_mapped,
                );
            }

            let bn = h_frame_data
                .frame_info
                .borders_noise
                .get((env_noise + 1) as usize)
                .copied()
                .unwrap_or(0);
            if i >= bn as i32 {
                if env_noise >= 0 {
                    // The noise floor data is stored in a row [noiseFloor1 noiseFloor2...].
                    noise_off += no_noise_bands.max(0) as usize;
                } else {
                    // leave trailing noise envelope of past frame
                    no_noise_bands = h_freq.n_nfb as i32;
                    no_sub_frame_bands = h_freq.n_sfb;
                    noise_src = NoiseSrc::Frame;
                    noise_off = 0;

                    low_subband = h_freq.low_subband as i32;
                    high_subband = h_freq.high_subband as i32;

                    no_subbands = high_subband - low_subband;
                    ov_high_subband = high_subband;
                    if high_subband < h_freq.ov_high_subband as i32 {
                        ov_high_subband = h_freq.ov_high_subband as i32;
                    }

                    tab_lo = h_freq.freq_band_table_lo;
                    tab_hi = h_freq.freq_band_table_hi;
                    tab_noise = h_freq.freq_band_table_noise;
                }
                env_noise += 1;
            }
        } else {
            // If the start-pos of the current envelope equals the stop pos of the current noise
            // envelope, increase the pointer (i.e. choose the next noise-floor).
            let bn = h_frame_data
                .frame_info
                .borders_noise
                .get((env_noise + 1) as usize)
                .copied()
                .unwrap_or(0);
            if borders[i as usize] == bn {
                // The noise floor data is stored in a row [noiseFloor1 noiseFloor2...].
                noise_off += no_noise_bands.max(0) as usize;
                env_noise += 1;
            }
        }
        if i == h_frame_data.frame_info.tran_env as i32 || i == h_sbr_cal_env.prev_tran_env {
            // attack
            no_noise_flag = 1;
            if !use_lp {
                smooth_length = 0; // No smoothing on attacks!
            }
        } else {
            no_noise_flag = 0;
            if !use_lp {
                // can become either 0 or 4
                smooth_length = ((1 - h_header_data.bs_data.smoothing_length as i32) << 2) as u8;
            }
        }

        let ns = (no_subbands.max(0) as usize).min(MAX_FREQ_COEFFS);

        // Energy estimation in transposed highband.
        if h_header_data.bs_data.interpol_freq != 0 {
            calc_nrg_per_subband(
                analys_buffer_real,
                if use_lp { None } else { Some(&*analys_buffer_imag) },
                low_subband,
                high_subband,
                start_pos,
                stop_pos,
                input_e,
                &mut nrgs.nrg_est,
                &mut nrgs.nrg_est_e,
            );
        } else {
            let table: &[u8] = if freq_res == 0 { &tab_lo } else { &tab_hi };
            calc_nrg_per_sfb(
                analys_buffer_real,
                if use_lp { None } else { Some(&*analys_buffer_imag) },
                no_sub_frame_bands[freq_res] as i32,
                table,
                start_pos,
                stop_pos,
                input_e,
                &mut nrgs.nrg_est,
                &mut nrgs.nrg_est_e,
            );
        }

        // Calculate subband gains
        {
            let table: &[u8] = if freq_res == 0 { &tab_lo } else { &tab_hi };
            let tbl = |j: usize| table.get(j).copied().unwrap_or(0) as i32;
            // Upper limit of the current noise floor band.
            let mut p_ui_noise = 1usize;
            let ui_noise = |p: usize| tab_noise.get(p).copied().unwrap_or(255) as i32;

            let mut p_noise_levels = noise_off;

            let nl = noise_at(noise_src, p_noise_levels) as i32;
            let mut tmp_noise = fx_sgl2fx_dbl((nl & MASK_M) as FixpSgl);
            let mut tmp_noise_e: i8 = ((nl & MASK_E) as u8 as i32 - NOISE_EXP_OFFSET) as i8;
            p_noise_levels += 1;

            let mut cc = 0usize;
            let mut c = 0usize;
            if pvc_mode > 0 {
                for j in 0..no_sub_frame_bands[freq_res] as usize {
                    let mut sine_present_flag: u8 = 0;
                    let li = tbl(j);
                    let ui = tbl(j + 1);

                    for _ in li..ui {
                        sine_present_flag |=
                            (i >= sine_mapped.get(cc).copied().unwrap_or(32) as i32) as u8;
                        cc += 1;
                    }

                    for k in li..ui {
                        let kk = (k - low_subband).clamp(0, MAX_FREQ_COEFFS as i32 - 1) as usize;
                        let ref_nrg = nrgs.nrg_ref[kk];
                        let ref_nrg_e = nrgs.nrg_ref_e[kk];

                        if k >= ui_noise(p_ui_noise) {
                            let nl = noise_at(noise_src, p_noise_levels) as i32;
                            tmp_noise = fx_sgl2fx_dbl((nl & MASK_M) as FixpSgl);
                            tmp_noise_e = ((nl & MASK_E) as i8 as i32 - NOISE_EXP_OFFSET) as i8;
                            p_noise_levels += 1;

                            p_ui_noise += 1;
                        }

                        fdk_assert!(k >= low_subband);

                        if use_lp {
                            if let Some(u) = use_alias_reduction.get_mut((k - low_subband) as usize)
                            {
                                *u = (sine_present_flag == 0) as u8;
                            }
                        }

                        if c >= MAX_FREQ_COEFFS {
                            continue;
                        }
                        nrgs.nrg_sine[c] = 0;
                        nrgs.nrg_sine_e[c] = 0;

                        calc_subband_gain(
                            ref_nrg,
                            ref_nrg_e,
                            &mut nrgs,
                            c,
                            tmp_noise,
                            tmp_noise_e,
                            sine_present_flag,
                            (i >= sine_mapped[c] as i32) as u8,
                            no_noise_flag,
                        );

                        c += 1;
                    }
                }
            } else {
                for j in 0..no_sub_frame_bands[freq_res] as usize {
                    let ienv = h_frame_data.i_envelope.get(p_ienv).copied().unwrap_or(0) as i32;
                    let ref_nrg = fx_sgl2fx_dbl((ienv & MASK_M) as FixpSgl);
                    let ref_nrg_e = ((ienv & MASK_E) as i8 as i32 - NRG_EXP_OFFSET) as i8;

                    let mut sine_present_flag: u8 = 0;
                    let li = tbl(j);
                    let ui = tbl(j + 1);

                    for _ in li..ui {
                        sine_present_flag |=
                            (i >= sine_mapped.get(cc).copied().unwrap_or(32) as i32) as u8;
                        cc += 1;
                    }

                    for k in li..ui {
                        if k >= ui_noise(p_ui_noise) {
                            let nl = noise_at(noise_src, p_noise_levels) as i32;
                            tmp_noise = fx_sgl2fx_dbl((nl & MASK_M) as FixpSgl);
                            tmp_noise_e = ((nl & MASK_E) as i8 as i32 - NOISE_EXP_OFFSET) as i8;
                            p_noise_levels += 1;

                            p_ui_noise += 1;
                        }

                        fdk_assert!(k >= low_subband);

                        if use_lp {
                            if let Some(u) = use_alias_reduction.get_mut((k - low_subband) as usize)
                            {
                                *u = (sine_present_flag == 0) as u8;
                            }
                        }

                        if c >= MAX_FREQ_COEFFS {
                            continue;
                        }
                        nrgs.nrg_sine[c] = 0;
                        nrgs.nrg_sine_e[c] = 0;

                        calc_subband_gain(
                            ref_nrg,
                            ref_nrg_e,
                            &mut nrgs,
                            c,
                            tmp_noise,
                            tmp_noise_e,
                            sine_present_flag,
                            (i >= sine_mapped[c] as i32) as u8,
                            no_noise_flag,
                        );

                        nrgs.nrg_ref[c] = ref_nrg;
                        nrgs.nrg_ref_e[c] = ref_nrg_e;

                        c += 1;
                    }
                    p_ienv += 1;
                }
            }
        }

        // Noise limiting

        for c in 0..h_freq.no_limiter_bands as usize {
            let mut accu: FixpDbl = 0;
            let mut accu_e: i8 = 0;
            let lim_lo = h_freq.limiter_band_table.get(c).copied().unwrap_or(0) as usize;
            let lim_hi = (h_freq.limiter_band_table.get(c + 1).copied().unwrap_or(0) as usize)
                .min(MAX_FREQ_COEFFS);

            let (sum_ref, sum_ref_e, mut max_gain, mut max_gain_e) =
                calc_avg_gain(&nrgs, lim_lo, lim_hi);

            // Multiply maxGain with limiterGain:
            let lg = h_header_data.bs_data.limiter_gains as usize & 3;
            max_gain = f_mult(max_gain, FDK_SBR_DECODER_SBR_LIM_GAINS_M[lg]);
            // The addition of maxGain_e and FDK_sbrDecoder_sbr_limGains_e[3] might yield values
            // greater than 127 which doesn't fit into an SCHAR! In these rare situations limit
            // maxGain_e to 127.
            let max_gain_lim_gain_sum_e =
                max_gain_e as i32 + FDK_SBR_DECODER_SBR_LIM_GAINS_E[lg] as i32;
            max_gain_e =
                if max_gain_lim_gain_sum_e > 127 { 127 } else { max_gain_lim_gain_sum_e as i8 };

            // Scale mantissa of MaxGain into range between 0.5 and 1:
            if max_gain == 0 {
                max_gain_e = -FRACT_BITS as i8;
            } else {
                let char_temp = count_leading_bits(max_gain) as i8;
                max_gain_e = max_gain_e.wrapping_sub(char_temp);
                max_gain <<= char_temp as i32;
            }

            if max_gain_e >= max_gain_limit_e {
                // upper limit (e.g. 96 dB)
                max_gain = fl2fxconst_dbl(0.5f32 as f64);
                max_gain_e = max_gain_limit_e;
            }

            // Every subband gain is compared to the scaled "average gain" and limited if
            // necessary:
            for k in lim_lo..lim_hi {
                if nrgs.nrg_gain_e[k] > max_gain_e
                    || (nrgs.nrg_gain_e[k] == max_gain_e && nrgs.nrg_gain[k] > max_gain)
                {
                    let (noise_amp, noise_amp_e) = fdk_divide_mant_exp(
                        max_gain,
                        max_gain_e,
                        nrgs.nrg_gain[k],
                        nrgs.nrg_gain_e[k],
                    );
                    nrgs.noise_level[k] = f_mult(nrgs.noise_level[k], noise_amp);
                    nrgs.noise_level_e[k] = nrgs.noise_level_e[k].wrapping_add(noise_amp_e);
                    nrgs.nrg_gain[k] = max_gain;
                    nrgs.nrg_gain_e[k] = max_gain_e;
                }
            }

            // -- Boost gain
            // Calculate and apply boost factor for each limiter band:
            // 1. Check how much energy would be present when using the limited gain
            // 2. Calculate boost factor by comparison with reference energy
            // 3. Apply boost factor to compensate for the energy loss due to limiting
            for k in lim_lo..lim_hi {
                // 1.a  Add energy of adjusted signal (using preliminary gain)
                let tmp = f_mult(nrgs.nrg_gain[k], nrgs.nrg_est[k]);
                let tmp_e = nrgs.nrg_gain_e[k].wrapping_add(nrgs.nrg_est_e[k]);
                (accu, accu_e) = fdk_add_mant_exp(tmp, tmp_e, accu, accu_e);

                // 1.b  Add sine energy (if present)
                if nrgs.nrg_sine[k] != 0 {
                    (accu, accu_e) =
                        fdk_add_mant_exp(nrgs.nrg_sine[k], nrgs.nrg_sine_e[k], accu, accu_e);
                } else {
                    // 1.c  Add noise energy (if present)
                    if no_noise_flag == 0 {
                        (accu, accu_e) = fdk_add_mant_exp(
                            nrgs.noise_level[k],
                            nrgs.noise_level_e[k],
                            accu,
                            accu_e,
                        );
                    }
                }
            }

            // 2.a  Calculate ratio of wanted energy and accumulated energy
            let mut boost_gain: FixpDbl;
            let mut boost_gain_e: i8;
            if accu == 0 {
                // If divisor is 0, limit quotient to +4 dB
                boost_gain = fl2fxconst_dbl(0.6279716f32 as f64);
                boost_gain_e = 2;
            } else {
                let mut div_e = 0;
                boost_gain = f_div_norm_fast(sum_ref, accu, &mut div_e);
                boost_gain_e = (sum_ref_e as i32 - accu_e as i32 + div_e) as i8;
            }

            // 2.b Result too high? --> Limit the boost factor to +4 dB
            if boost_gain_e > 3
                || (boost_gain_e == 2 && boost_gain > fl2fxconst_dbl(0.6279716f32 as f64))
                || (boost_gain_e == 3 && boost_gain > fl2fxconst_dbl(0.3139858f32 as f64))
            {
                boost_gain = fl2fxconst_dbl(0.6279716f32 as f64);
                boost_gain_e = 2;
            }
            // 3.  Multiply all signal components with the boost factor
            for k in lim_lo..lim_hi {
                nrgs.nrg_gain[k] = f_mult_div2(nrgs.nrg_gain[k], boost_gain);
                nrgs.nrg_gain_e[k] = (nrgs.nrg_gain_e[k] as i32 + boost_gain_e as i32 + 1) as i8;

                nrgs.nrg_sine[k] = f_mult_div2(nrgs.nrg_sine[k], boost_gain);
                nrgs.nrg_sine_e[k] = (nrgs.nrg_sine_e[k] as i32 + boost_gain_e as i32 + 1) as i8;

                nrgs.noise_level[k] = f_mult_div2(nrgs.noise_level[k], boost_gain);
                nrgs.noise_level_e[k] =
                    (nrgs.noise_level_e[k] as i32 + boost_gain_e as i32 + 1) as i8;
            }
        }
        // End of noise limiting

        if use_lp {
            let lo = (low_subband.max(0) as usize).min(degree_alias.len());
            aliasing_reduction(&degree_alias[lo..], &mut nrgs, &use_alias_reduction, no_subbands);
        }

        // For the timeslots within the range for the output frame, use the same scale for the
        // noise levels. Drawback: If the envelope exceeds the frame border, the noise levels
        // will have to be rescaled later to fit final_e of the gain-values.
        noise_e = if start_pos < no_cols { adj_e } else { final_e };

        if start_pos >= no_cols {
            let diff = h_sbr_cal_env.filt_buffer_noise_e as i32 - noise_e as i32;
            if diff > 0 {
                let s = get_scalefactor(&h_sbr_cal_env.filt_buffer_noise[..ns]);
                if diff > s {
                    final_e = (final_e as i32 + diff - s) as i8;
                    noise_e = final_e;
                }
            }
        }

        // Convert energies to amplitude levels
        for k in 0..ns {
            fdk_sqrt_mant_exp(&mut nrgs.nrg_sine[k], &mut nrgs.nrg_sine_e[k], Some(noise_e));
            fdk_sqrt_mant_exp(&mut nrgs.nrg_gain[k], &mut nrgs.nrg_gain_e[k], None);
            fdk_sqrt_mant_exp(&mut nrgs.noise_level[k], &mut nrgs.noise_level_e[k], Some(noise_e));
        }

        // Apply calculated gains and adaptive noise

        // assembleHfSignals()
        {
            let mut filt_buffer_noise_shift = 0;

            // Initialize smoothing buffers with the first valid values
            if h_sbr_cal_env.start_up != 0 {
                if !use_lp {
                    h_sbr_cal_env.filt_buffer_noise_e = noise_e;

                    h_sbr_cal_env.filt_buffer_e[..ns].copy_from_slice(&nrgs.nrg_gain_e[..ns]);
                    h_sbr_cal_env.filt_buffer_noise[..ns].copy_from_slice(&nrgs.noise_level[..ns]);
                    h_sbr_cal_env.filt_buffer[..ns].copy_from_slice(&nrgs.nrg_gain[..ns]);
                }
                h_sbr_cal_env.start_up = 0;
            }

            if !use_lp {
                equalize_filt_buffer_exp(
                    &mut h_sbr_cal_env.filt_buffer,   // buffered
                    &mut h_sbr_cal_env.filt_buffer_e, // buffered
                    &mut nrgs.nrg_gain,               // current
                    &mut nrgs.nrg_gain_e,             // current
                    ns as i32,
                );

                // Adapt exponent of buffered noise levels to the current exponent so they can
                // easily be smoothed
                if (h_sbr_cal_env.filt_buffer_noise_e as i32 - noise_e as i32) >= 0 {
                    let shift = fix_min(
                        DFRACT_BITS - 1,
                        h_sbr_cal_env.filt_buffer_noise_e as i32 - noise_e as i32,
                    );
                    for v in h_sbr_cal_env.filt_buffer_noise[..ns].iter_mut() {
                        *v <<= shift;
                    }
                } else {
                    let shift = fix_min(
                        DFRACT_BITS - 1,
                        -(h_sbr_cal_env.filt_buffer_noise_e as i32 - noise_e as i32),
                    );
                    for v in h_sbr_cal_env.filt_buffer_noise[..ns].iter_mut() {
                        *v >>= shift;
                    }
                }

                h_sbr_cal_env.filt_buffer_noise_e = noise_e;
            }

            // find best scaling!
            let mut scale_change = -(DFRACT_BITS - 1);
            for k in 0..ns {
                scale_change = fix_max(scale_change, nrgs.nrg_gain_e[k] as i32);
            }
            let mut sc_change = if start_pos < no_cols {
                adj_e as i32 - input_e as i32
            } else {
                final_e as i32 - input_e as i32
            };

            if (scale_change - sc_change + 1) < 0 {
                scale_change -= scale_change - sc_change + 1;
            }

            scale_change = (scale_change - sc_change) + 1;

            for k in 0..ns {
                let sc = scale_change - nrgs.nrg_gain_e[k] as i32 + (sc_change - 1);
                // sc >= 0 (see the computation of scale_change)
                nrgs.nrg_gain[k] =
                    nrgs.nrg_gain[k].wrapping_shr(fix_min(sc, DFRACT_BITS - 1) as u32);
                nrgs.nrg_gain_e[k] = (nrgs.nrg_gain_e[k] as i32 + sc) as i8;
            }

            if !use_lp {
                for k in 0..ns {
                    let sc = scale_change - h_sbr_cal_env.filt_buffer_e[k] as i32 + (sc_change - 1);
                    h_sbr_cal_env.filt_buffer[k] = h_sbr_cal_env.filt_buffer[k]
                        .wrapping_shr(fix_min(sc, DFRACT_BITS - 1) as u32);
                }
            }

            for j in start_pos..stop_pos {
                // This timeslot is located within the first part of the processing buffer and
                // will be fed into the QMF-synthesis for the current frame.
                //     adj_e - input_e
                // This timeslot will not yet be fed into the QMF so we do not care about the
                // adj_e.
                //     sc_change = final_e - input_e
                if j == no_cols && start_pos < no_cols {
                    let mut shift = noise_e as i32 - final_e as i32;
                    if !use_lp {
                        // shifting of h_sbr_cal_env->filtBufferNoise[k] will be applied in
                        // function adjustTimeSlotHQ()
                        filt_buffer_noise_shift = shift;
                    }
                    if shift >= 0 {
                        shift = fix_min(DFRACT_BITS - 1, shift);
                        for k in 0..ns {
                            nrgs.nrg_sine[k] <<= shift;
                            nrgs.noise_level[k] <<= shift;
                        }
                    } else {
                        shift = fix_min(DFRACT_BITS - 1, -shift);
                        for k in 0..ns {
                            nrgs.nrg_sine[k] >>= shift;
                            nrgs.noise_level[k] >>= shift;
                        }
                    }

                    // update noise scaling
                    noise_e = final_e;
                    if !use_lp {
                        h_sbr_cal_env.filt_buffer_noise_e = noise_e; // scaling value unused!
                    }

                    // update gain buffer
                    sc_change -= final_e as i32 - input_e as i32;

                    if sc_change < 0 {
                        // -sc_change can be 32 or more: register shift (amount modulo 32)
                        for k in 0..ns {
                            nrgs.nrg_gain[k] = nrgs.nrg_gain[k].wrapping_shr((-sc_change) as u32);
                            nrgs.nrg_gain_e[k] = (nrgs.nrg_gain_e[k] as i32 - sc_change) as i8;
                        }
                        if !use_lp {
                            for k in 0..ns {
                                h_sbr_cal_env.filt_buffer[k] =
                                    h_sbr_cal_env.filt_buffer[k].wrapping_shr((-sc_change) as u32);
                            }
                        }
                    } else {
                        scale_change += sc_change;
                    }
                }

                let ju = j as usize;
                if !use_lp {
                    // Prevent the smoothing filter from running on constant levels
                    let smooth_ratio: FixpSgl = if j - start_pos < smooth_length as i32 {
                        FDK_SBR_DECODER_SBR_SMOOTH_FILTER[(j - start_pos) as usize]
                    } else {
                        0
                    };

                    let lo = (low_subband.max(0) as usize).min(64);
                    let (re, im) =
                        (&mut analys_buffer_real[ju][lo..], &mut analys_buffer_imag[ju][lo..]);
                    if i_tes_enable != 0 {
                        // adjustTimeSlotHQ() without adding of additional harmonics
                        adjust_time_slot_hq_gain_and_noise(
                            re,
                            im,
                            h_sbr_cal_env,
                            &nrgs,
                            low_subband,
                            ns as i32,
                            f_min(scale_change, DFRACT_BITS - 1),
                            smooth_ratio,
                            no_noise_flag,
                            filt_buffer_noise_shift,
                        );
                    } else {
                        adjust_time_slot_hq(
                            re,
                            im,
                            h_sbr_cal_env,
                            &nrgs,
                            low_subband,
                            ns as i32,
                            f_min(scale_change, DFRACT_BITS - 1),
                            smooth_ratio,
                            no_noise_flag,
                            filt_buffer_noise_shift,
                        );
                    }
                } else {
                    fdk_assert!(i_tes_enable == 0); // not supported
                    if flags & SBRDEC_ELD_GRID != 0 {
                        // FDKmemset(analysBufferReal[j], 0, 64 * sizeof(FIXP_DBL));
                        adjust_time_slot_eld_grid(
                            analys_buffer_real[ju],
                            low_subband,
                            &nrgs,
                            &mut h_sbr_cal_env.harm_index,
                            ns as i32,
                            f_min(scale_change, DFRACT_BITS - 1),
                            no_noise_flag,
                            &mut h_sbr_cal_env.phase_index,
                            f_max(
                                15i32
                                    .wrapping_sub(adj_e as i32)
                                    .wrapping_sub(sbr_scale_factor.lb_scale),
                                -(DFRACT_BITS - 1),
                            ),
                        );
                    } else {
                        adjust_time_slot_lc(
                            analys_buffer_real[ju],
                            low_subband,
                            &nrgs,
                            &mut h_sbr_cal_env.harm_index,
                            ns as i32,
                            f_min(scale_change, DFRACT_BITS - 1),
                            no_noise_flag,
                            &mut h_sbr_cal_env.phase_index,
                        );
                    }
                }
                // In case the envelope spans accross the no_cols border both exponents are
                // needed. nrgGain_e[0...(noSubbands-1)] are equalized by
                // equalizeFiltBufferExp()
                nrgs.exponent[if j < no_cols { 0 } else { 1 }] =
                    ((15 - sbr_scale_factor.hb_scale) + nrgs.nrg_gain_e[0] as i32 + 1
                        - scale_change) as i8;
            } // for

            if i_tes_enable != 0 {
                let gamma =
                    h_frame_data.inter_temp_shape_mode.get(i as usize).copied().unwrap_or(0);
                apply_inter_tes(
                    analys_buffer_real,
                    analys_buffer_imag,
                    sbr_scale_factor,
                    &nrgs.exponent,
                    h_header_data.time_step as i32,
                    start_pos,
                    stop_pos,
                    low_subband,
                    no_subbands,
                    gamma, // frameData->interTempShapeMode[env]
                );

                // add additional harmonics
                for j in start_pos..stop_pos {
                    // match exponent of additional harmonics to scale change of QMF data caused
                    // by apply_inter_tes()
                    let mut scale_change = 0;

                    if start_pos <= no_cols && stop_pos > no_cols {
                        // Scaling of analysBuffers was potentially changed within this envelope.
                        // The pNrgs->nrgSine_e match the second part of the envelope. For
                        // (j<=no_cols) the exponent of the sine energies has to be adapted.
                        scale_change = nrgs.exponent[1] as i32 - nrgs.exponent[0] as i32;
                    }

                    let ju = j as usize;
                    let lo = (low_subband.max(0) as usize).min(64);
                    adjust_time_slot_hq_add_harmonics(
                        &mut analys_buffer_real[ju][lo..],
                        &mut analys_buffer_imag[ju][lo..],
                        h_sbr_cal_env,
                        &nrgs,
                        low_subband,
                        ns as i32,
                        -i_tes_scale_change + if j < no_cols { scale_change } else { 0 },
                    );
                }
            }

            if !use_lp {
                // Update time-smoothing-buffers for gains and noise levels. The gains and the
                // noise values of the current envelope are copied into the buffer. This has to
                // be done at the end of each envelope as the values are required for a smooth
                // transition to the next envelope.
                h_sbr_cal_env.filt_buffer[..ns].copy_from_slice(&nrgs.nrg_gain[..ns]);
                h_sbr_cal_env.filt_buffer_e[..ns].copy_from_slice(&nrgs.nrg_gain_e[..ns]);
                h_sbr_cal_env.filt_buffer_noise[..ns].copy_from_slice(&nrgs.noise_level[..ns]);
            }
        }
        i += 1;
    }

    // adapt adj_e to the scale change caused by apply_inter_tes()
    adj_e = (adj_e as i32 + i_tes_scale_change) as i8;

    // Rescale output samples
    {
        // Determine headroom in old adjusted samples
        let max_val = max_subband_sample(
            analys_buffer_real,
            if use_lp { None } else { Some(&*analys_buffer_imag) },
            low_subband,
            ov_high_subband,
            0,
            first_start,
        );

        let ov_reserve = f_norm(max_val);

        // Determine headroom in new adjusted samples
        let max_val = max_subband_sample(
            analys_buffer_real,
            if use_lp { None } else { Some(&*analys_buffer_imag) },
            low_subband,
            high_subband,
            first_start,
            no_cols,
        );

        let reserve = f_norm(max_val);

        // Determine common output exponent
        output_e = f_max(ov_adj_e as i32 - ov_reserve, adj_e as i32 - reserve) as i8;

        // Rescale old samples
        rescale_subband_samples(
            analys_buffer_real,
            if use_lp { None } else { Some(&mut *analys_buffer_imag) },
            low_subband,
            ov_high_subband,
            0,
            first_start,
            ov_adj_e as i32 - output_e as i32,
        );

        // Rescale new samples
        rescale_subband_samples(
            analys_buffer_real,
            if use_lp { None } else { Some(&mut *analys_buffer_imag) },
            low_subband,
            high_subband,
            first_start,
            no_cols,
            adj_e as i32 - output_e as i32,
        );
    }

    // Update hb_scale
    sbr_scale_factor.hb_scale = 15 - output_e as i32; // EXP2SCALE(output_e)

    // Save the current final exponent for the next frame:
    // adapt final_e to the scale change caused by apply_inter_tes()
    sbr_scale_factor.ov_hb_scale = 15 - (final_e as i32 + i_tes_scale_change); // EXP2SCALE

    // We need to remember to the next frame that the transient will occur in the first envelope
    // (if tranEnv == nEnvelopes).
    if h_frame_data.frame_info.tran_env as i32 == h_frame_data.frame_info.n_envelopes as i32 {
        h_sbr_cal_env.prev_tran_env = 0;
    } else {
        h_sbr_cal_env.prev_tran_env = -1;
    }

    if pvc_mode > 0 {
        // Not more than just the last noise envelope reaches into the next PVC frame! This
        // should be true because bs_noise_position is <= 15
        let nne = h_frame_data.frame_info.n_noise_envelopes as usize;
        fdk_assert!(
            (h_frame_data.frame_info.borders_noise[nne.wrapping_sub(1).min(2)] as usize)
                < PVC_NTIMESLOT
        );
        if h_frame_data.frame_info.borders_noise[nne.min(2)] as usize > PVC_NTIMESLOT {
            fdk_assert!(
                noise_src == NoiseSrc::Frame
                    && noise_off == nne.wrapping_sub(1).wrapping_mul(no_noise_bands as usize)
            );
            h_sbr_cal_env.prev_n_nfb = no_noise_bands as u8;

            h_sbr_cal_env.prev_n_sfb[0] = no_sub_frame_bands[0];
            h_sbr_cal_env.prev_n_sfb[1] = no_sub_frame_bands[1];

            h_sbr_cal_env.prev_lo_subband = low_subband as u8;
            h_sbr_cal_env.prev_hi_subband = high_subband as u8;
            h_sbr_cal_env.prev_ov_high_subband = ov_high_subband as u8;

            let n0 = (no_sub_frame_bands[0] as usize + 1).min(tab_lo.len());
            h_sbr_cal_env.prev_freq_band_table_lo[..n0].copy_from_slice(&tab_lo[..n0]);
            let n1 = (no_sub_frame_bands[1] as usize + 1).min(tab_hi.len());
            h_sbr_cal_env.prev_freq_band_table_hi[..n1].copy_from_slice(&tab_hi[..n1]);
            h_sbr_cal_env.prev_freq_band_table_noise = h_freq.freq_band_table_noise;

            for k in 0..MAX_NOISE_COEFFS {
                h_sbr_cal_env.prev_sbr_noise_floor_level[k] = noise_at(noise_src, noise_off + k);
            }
        }
    }
}

/// `createSbrEnvelopeCalc()`: creates an envelope calculator instance; the frequency band
/// tables of the header are calculated for channel 0.
pub fn create_sbr_envelope_calc(
    hs: &mut SbrCalculateEnvelope,
    h_header_data: &mut SbrHeaderData,
    chan: i32,
    flags: u32,
) -> SbrError {
    let mut err = SBRDEC_OK;

    // Clear previous missing harmonics flags
    hs.harm_flags_prev = [0; ADD_HARMONICS_FLAGS_SIZE];
    hs.harm_flags_prev_active = [0; ADD_HARMONICS_FLAGS_SIZE];
    hs.harm_index = 0;

    hs.prev_sbr_noise_floor_level = [0; MAX_NOISE_COEFFS];
    hs.prev_n_nfb = 0;
    hs.prev_freq_band_table_noise = [0; MAX_NOISE_COEFFS + 1];
    hs.sinusoidal_position_prev = 0;

    // Setup pointers for time smoothing. The buffer itself will be initialized later triggered
    // by the startUp-flag.
    hs.prev_tran_env = -1;

    // initialization
    reset_sbr_envelope_calc(hs);

    if chan == 0 {
        // do this only once
        err = crate::sbrdec_freq_sca::reset_freq_band_tables(h_header_data, flags);
    }

    err
}

/// `deleteSbrEnvelopeCalc()`
pub fn delete_sbr_envelope_calc(_hs: &mut SbrCalculateEnvelope) -> i32 {
    0
}

/// `resetSbrEnvelopeCalc()`: resets the envelope calculator on a change of configuration.
pub fn reset_sbr_envelope_calc(h_cal_env: &mut SbrCalculateEnvelope) {
    h_cal_env.phase_index = 0;

    // Noise exponent needs to be reset because the output exponent for the next frame depends
    // on it
    h_cal_env.filt_buffer_noise_e = 0;

    h_cal_env.start_up = 1;
}

/// `equalizeFiltBufferExp()`: equalizes the exponents of the buffered gain values and the new
/// ones (before the FIR smoothing filter).
pub fn equalize_filt_buffer_exp(
    filt_buffer: &mut [FixpDbl],
    filt_buffer_e: &mut [i8],
    nrg_gain: &mut [FixpDbl],
    nrg_gain_e: &mut [i8],
    subbands: i32,
) {
    for band in 0..subbands.max(0) as usize {
        let mut diff = nrg_gain_e[band] as i32 - filt_buffer_e[band] as i32;
        if diff > 0 {
            // Compensate for the scale change by shifting the mantissa.
            filt_buffer[band] >>= f_min(diff, DFRACT_BITS - 1);
            // New gain is bigger, use its exponent
            filt_buffer_e[band] = (filt_buffer_e[band] as i32 + diff) as i8;
        } else if diff < 0 {
            // The buffered gains seem to be larger, but maybe there are some unused bits left in
            // the mantissa
            let reserve = cnt_leading_zeros(fixp_abs(filt_buffer[band])) - 1;

            if (-diff) <= reserve {
                // There is enough space in the buffered mantissa so that we can take the new
                // exponent as common.
                filt_buffer[band] <<= -diff;
                filt_buffer_e[band] = (filt_buffer_e[band] as i32 + diff) as i8;
            // becomes equal to *ptrNewExp
            } else {
                // Shift the mantissa as far as possible: (reserve is -1 for the mantissa
                // MINVAL_DBL: register shift, amount modulo 32)
                filt_buffer[band] = filt_buffer[band].wrapping_shl(reserve as u32);
                // Compensate in the exponent:
                filt_buffer_e[band] = (filt_buffer_e[band] as i32 - reserve) as i8;

                // For the remaining difference, change the new gain value
                diff = -(reserve + diff);
                nrg_gain[band] >>= f_min(diff, DFRACT_BITS - 1);
                nrg_gain_e[band] = (nrg_gain_e[band] as i32 + diff) as i8;
            }
        }
    }
}

/// `rescaleSubbandSamples()`: shifts the subband samples of the given time and frequency range
/// by `shift` bits (left for positive values). `im` is `None` in low power mode.
pub fn rescale_subband_samples(
    re: &mut [&mut [FixpDbl]],
    mut im: Option<&mut [&mut [FixpDbl]]>,
    low_subband: i32,
    high_subband: i32,
    start_pos: i32,
    next_pos: i32,
    shift: i32,
) {
    let width = high_subband - low_subband;

    if width > 0 && shift != 0 {
        let lo = low_subband.max(0) as usize;
        let hi = high_subband.clamp(0, 64) as usize;
        for l in start_pos.max(0)..next_pos {
            let l = l as usize;
            if let Some(s) = re.get_mut(l).and_then(|s| s.get_mut(lo..hi)) {
                scale_values(s, shift);
            }
            if let Some(im) = im.as_deref_mut() {
                if let Some(s) = im.get_mut(l).and_then(|s| s.get_mut(lo..hi)) {
                    scale_values(s, shift);
                }
            }
        }
    }
}

/// `maxSubbandSample()`: the maximum magnitude of the subband samples in the given time and
/// frequency range (for the headroom calculation). `im` is `None` in low power mode.
///
/// The straight port, kept as the reference of the optimized [`max_subband_sample`].
pub fn max_subband_sample_straight(
    re: &[&mut [FixpDbl]],
    im: Option<&[&mut [FixpDbl]]>,
    low_subband: i32,
    high_subband: i32,
    start_pos: i32,
    next_pos: i32,
) -> FixpDbl {
    let mut max_val: FixpDbl = 0;
    let width = (high_subband - low_subband) as u32;

    fdk_assert!(width <= 64);

    // width is unsigned: for a negative width the reference has no defined result; the port
    // reads nothing then.
    if width > 0 && width <= 64 {
        let lo = low_subband.max(0) as usize;
        let hi = (lo + width as usize).min(64);
        for l in start_pos.max(0)..next_pos {
            let l = l as usize;
            if let Some(s) = re.get(l) {
                for &t in s.get(lo..hi).unwrap_or(&[]) {
                    max_val |= t ^ (t >> (DFRACT_BITS - 1));
                }
            }
            if let Some(im) = im {
                if let Some(s) = im.get(l) {
                    for &t in s.get(lo..hi).unwrap_or(&[]) {
                        max_val |= t ^ (t >> (DFRACT_BITS - 1));
                    }
                }
            }
        }
    }

    if max_val > 0 {
        // For negative input values, maxVal is too small by 1. Add 1 only when necessary: if
        // maxVal is a power of 2
        let lower_pow2 = 1i32 << (DFRACT_BITS - 1 - cnt_leading_zeros(max_val));
        if max_val == lower_pow2 {
            max_val += 1;
        }
    }

    max_val
}

/// `maxSubbandSample()`, optimized (see [`max_subband_sample_straight`]): on aarch64 the
/// magnitudes `t ^ (t >> 31)` of all slots are ORed into one NEON accumulator (four lanes,
/// two registers), the rest of each row into a scalar one; OR is associative and commutative.
pub fn max_subband_sample(
    re: &[&mut [FixpDbl]],
    im: Option<&[&mut [FixpDbl]]>,
    low_subband: i32,
    high_subband: i32,
    start_pos: i32,
    next_pos: i32,
) -> FixpDbl {
    #[cfg(target_arch = "aarch64")]
    {
        let width = (high_subband - low_subband) as u32;
        fdk_assert!(width <= 64);
        let mut max_val: FixpDbl = 0;
        if width > 0 && width <= 64 {
            let lo = low_subband.max(0) as usize;
            let hi = (lo + width as usize).min(64);
            let mut acc = neon_max::Acc::new();
            for l in start_pos.max(0)..next_pos {
                let l = l as usize;
                if let Some(s) = re.get(l) {
                    acc.add(s.get(lo..hi).unwrap_or(&[]));
                }
                if let Some(im) = im {
                    if let Some(s) = im.get(l) {
                        acc.add(s.get(lo..hi).unwrap_or(&[]));
                    }
                }
            }
            max_val = acc.get();
        }
        if max_val > 0 {
            // For negative input values, maxVal is too small by 1. Add 1 only when necessary:
            // if maxVal is a power of 2
            let lower_pow2 = 1i32 << (DFRACT_BITS - 1 - cnt_leading_zeros(max_val));
            if max_val == lower_pow2 {
                max_val += 1;
            }
        }
        max_val
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
        if fdk_aac_rs_fdk::simd_x86::avx2() {
            // The same as on aarch64 with the AVX2 accumulator.
            let width = (high_subband - low_subband) as u32;
            fdk_assert!(width <= 64);
            let mut max_val: FixpDbl = 0;
            if width > 0 && width <= 64 {
                let lo = low_subband.max(0) as usize;
                let hi = (lo + width as usize).min(64);
                // SAFETY: the CPU has AVX2.
                unsafe {
                    let mut acc = x86::zero();
                    for l in start_pos.max(0)..next_pos {
                        let l = l as usize;
                        if let Some(s) = re.get(l) {
                            max_val |= x86::or_mag(&mut acc, s.get(lo..hi).unwrap_or(&[]));
                        }
                        if let Some(im) = im {
                            if let Some(s) = im.get(l) {
                                max_val |= x86::or_mag(&mut acc, s.get(lo..hi).unwrap_or(&[]));
                            }
                        }
                    }
                    max_val |= x86::or_lanes(acc);
                }
            }
            if max_val > 0 {
                let lower_pow2 = 1i32 << (DFRACT_BITS - 1 - cnt_leading_zeros(max_val));
                if max_val == lower_pow2 {
                    max_val += 1;
                }
            }
            return max_val;
        }
        max_subband_sample_straight(re, im, low_subband, high_subband, start_pos, next_pos)
    }
}

/// The OR accumulator of [`max_subband_sample`] (aarch64).
#[cfg(target_arch = "aarch64")]
mod neon_max {
    use super::FixpDbl;
    use core::arch::aarch64::*;

    pub(super) struct Acc(int32x4_t, int32x4_t, FixpDbl);

    impl Acc {
        #[inline(always)]
        pub(super) fn new() -> Self {
            // SAFETY: NEON is part of aarch64.
            unsafe { Acc(vdupq_n_s32(0), vdupq_n_s32(0), 0) }
        }

        /// ORs the magnitudes of `x` into the accumulator.
        #[inline(always)]
        pub(super) fn add(&mut self, x: &[FixpDbl]) {
            let n8 = x.len() & !7;
            // SAFETY: NEON is part of aarch64; the loads read x[k..k + 8] with k + 8 <= x.len().
            unsafe {
                let mag = |v: int32x4_t| veorq_s32(v, vshrq_n_s32::<31>(v));
                let p = x.as_ptr();
                let mut k = 0;
                while k < n8 {
                    self.0 = vorrq_s32(self.0, mag(vld1q_s32(p.add(k))));
                    self.1 = vorrq_s32(self.1, mag(vld1q_s32(p.add(k + 4))));
                    k += 8;
                }
            }
            for &t in &x[n8..] {
                self.2 |= t ^ (t >> 31);
            }
        }

        #[inline(always)]
        pub(super) fn get(&self) -> FixpDbl {
            // SAFETY: NEON is part of aarch64.
            unsafe {
                let v = vorrq_s32(self.0, self.1);
                let v = vorr_s32(vget_low_s32(v), vget_high_s32(v));
                vget_lane_s32::<0>(v) | vget_lane_s32::<1>(v) | self.2
            }
        }
    }
}

/// `calcNrgPerSubband()`: the mean energy of each filter-bank channel for the duration of the
/// current envelope (used when interpolFreq is true).
///
/// The straight port, kept as the reference of the optimized [`calc_nrg_per_subband`].
#[allow(clippy::too_many_arguments)]
pub fn calc_nrg_per_subband_straight(
    analys_buffer_real: &[&mut [FixpDbl]],
    analys_buffer_imag: Option<&[&mut [FixpDbl]]>,
    low_subband: i32,
    high_subband: i32,
    start_pos: i32,
    next_pos: i32,
    frame_exp: i8,
    nrg_est: &mut [FixpDbl],
    nrg_est_e: &mut [i8],
) {
    // Divide by width of envelope later:
    let inv_width = fx_dbl2fx_sgl(get_inv_int(next_pos - start_pos));
    // The common exponent needs to be doubled because all mantissas are squared:
    let frame_exp = (frame_exp as i32) << 1;

    for (out, k) in (low_subband.max(0)..high_subband.min(64)).enumerate() {
        let ku = k as usize;
        let mut buffer_real = [0 as FixpDbl; MAX_COLS + MAX_OV_COLS];
        let mut buffer_imag = [0 as FixpDbl; MAX_COLS + MAX_OV_COLS];
        let mut max_val: FixpDbl = 0;
        let (s, e) =
            (start_pos.max(0) as usize, (next_pos.max(0) as usize).min(MAX_COLS + MAX_OV_COLS));

        if let Some(im) = analys_buffer_imag {
            for l in s..e {
                buffer_imag[l] = im[l][ku];
                max_val |= buffer_imag[l] ^ (buffer_imag[l] >> (DFRACT_BITS - 1));
                buffer_real[l] = analys_buffer_real[l][ku];
                max_val |= buffer_real[l] ^ (buffer_real[l] >> (DFRACT_BITS - 1));
            }
        } else {
            for l in s..e {
                buffer_real[l] = analys_buffer_real[l][ku];
                max_val |= buffer_real[l] ^ (buffer_real[l] >> (DFRACT_BITS - 1));
            }
        }

        if out >= nrg_est.len() {
            break;
        }
        if max_val != 0 {
            // If the accu does not provide enough overflow bits, we cannot shift the samples up
            // to the limit. Instead, keep up to 3 free bits in each sample, i.e. up to 6 bits
            // after calculation of square. Please note the comment on saturated arithmetic above!
            let mut pre_shift = (cnt_leading_zeros(max_val) - 1) as i8;
            pre_shift -= SHIFT_BEFORE_SQUARE as i8;

            // Limit preShift to a maximum value to prevent accumulator overflow in exceptional
            // situations where the signal in the analysis-buffer is very small (small maxVal).
            pre_shift = f_min(pre_shift, 25);

            let mut accu: FixpDbl = 0;
            if pre_shift >= 0 {
                if analys_buffer_imag.is_some() {
                    for l in s..e {
                        let temp1 = buffer_real[l] << pre_shift as i32;
                        let temp2 = buffer_imag[l] << pre_shift as i32;
                        accu = f_pow2_add_div2(accu, temp1);
                        accu = f_pow2_add_div2(accu, temp2);
                    }
                } else {
                    for l in s..e {
                        let temp = buffer_real[l] << pre_shift as i32;
                        accu = f_pow2_add_div2(accu, temp);
                    }
                }
            } else {
                // if negative shift value
                let negpre_shift = -(pre_shift as i32);
                if analys_buffer_imag.is_some() {
                    for l in s..e {
                        let temp1 = buffer_real[l] >> negpre_shift;
                        let temp2 = buffer_imag[l] >> negpre_shift;
                        accu = f_pow2_add_div2(accu, temp1);
                        accu = f_pow2_add_div2(accu, temp2);
                    }
                } else {
                    for l in s..e {
                        let temp = buffer_real[l] >> negpre_shift;
                        accu = f_pow2_add_div2(accu, temp);
                    }
                }
            }
            accu <<= 1;

            // Convert double precision to Mantissa/Exponent:
            let mut shift = f_norm(accu) as i8;
            let sum = accu << shift as i32;

            // Divide by width of envelope and apply frame scale:
            nrg_est[out] = f_mult(sum, inv_width);
            shift = (shift as i32 + 2 * pre_shift as i32) as i8;
            if analys_buffer_imag.is_some() {
                nrg_est_e[out] = (frame_exp - shift as i32) as i8;
            } else {
                nrg_est_e[out] = (frame_exp - shift as i32 + 1) as i8; // +1 due to missing imag. part
            }
        } else {
            // Prevent a zero-mantissa-number from being misinterpreted due to its exponent.
            nrg_est[out] = 0;
            nrg_est_e[out] = 0;
        }
    }
}

/// `calcNrgPerSubband()`, optimized (see [`calc_nrg_per_subband_straight`]). On aarch64 four
/// subbands are processed at a time with NEON, slot by slot (the rows of the QMF buffer are
/// contiguous): the maximum magnitude per subband, then the sum of the squares with the
/// per-subband pre-shift (`sshl`: left shifts by `0..=25` wrap as `<<`, right shifts by `1..=4`
/// are arithmetic as `>>`; `fPow2Div2` is the high half of `smull`). The sums wrap, so their
/// order does not matter. The remaining subbands, other targets and buffers that do not cover
/// the range use the straight port.
#[allow(clippy::too_many_arguments)]
pub fn calc_nrg_per_subband(
    analys_buffer_real: &[&mut [FixpDbl]],
    analys_buffer_imag: Option<&[&mut [FixpDbl]]>,
    low_subband: i32,
    high_subband: i32,
    start_pos: i32,
    next_pos: i32,
    frame_exp: i8,
    nrg_est: &mut [FixpDbl],
    nrg_est_e: &mut [i8],
) {
    #[cfg(target_arch = "aarch64")]
    {
        let lo = low_subband.max(0);
        let n_bands = (high_subband.min(64) - lo).max(0) as usize;
        let n_bands = n_bands.min(nrg_est.len()).min(nrg_est_e.len());
        let n4 = n_bands & !3;
        let (s, e) =
            (start_pos.max(0) as usize, (next_pos.max(0) as usize).min(MAX_COLS + MAX_OV_COLS));
        let rows_ok = |b: &[&mut [FixpDbl]]| {
            s >= e || (e <= b.len() && b[s..e].iter().all(|r| r.len() >= lo as usize + n4))
        };
        if n4 > 0 && rows_ok(analys_buffer_real) && analys_buffer_imag.is_none_or(rows_ok) {
            let inv_width = fx_dbl2fx_sgl(get_inv_int(next_pos - start_pos));
            neon_nrg::per_subband(
                analys_buffer_real,
                analys_buffer_imag,
                lo as usize,
                n4,
                s..e,
                (frame_exp as i32) << 1,
                inv_width,
                nrg_est,
                nrg_est_e,
            );
            if n4 < n_bands || (high_subband.min(64) - lo) as usize > n_bands {
                calc_nrg_per_subband_straight(
                    analys_buffer_real,
                    analys_buffer_imag,
                    lo + n4 as i32,
                    high_subband,
                    start_pos,
                    next_pos,
                    frame_exp,
                    &mut nrg_est[n4..],
                    &mut nrg_est_e[n4..],
                );
            }
            return;
        }
    }
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    if fdk_aac_rs_fdk::simd_x86::avx2() {
        // The same as on aarch64, eight subbands at a time.
        let lo = low_subband.max(0);
        let n_bands = (high_subband.min(64) - lo).max(0) as usize;
        let n_bands = n_bands.min(nrg_est.len()).min(nrg_est_e.len());
        let n8 = n_bands & !7;
        let (s, e) =
            (start_pos.max(0) as usize, (next_pos.max(0) as usize).min(MAX_COLS + MAX_OV_COLS));
        let rows_ok = |b: &[&mut [FixpDbl]]| {
            s >= e || (e <= b.len() && b[s..e].iter().all(|r| r.len() >= lo as usize + n8))
        };
        if n8 > 0 && rows_ok(analys_buffer_real) && analys_buffer_imag.is_none_or(rows_ok) {
            let inv_width = fx_dbl2fx_sgl(get_inv_int(next_pos - start_pos));
            // SAFETY: the CPU has AVX2.
            unsafe {
                x86::per_subband(
                    analys_buffer_real,
                    analys_buffer_imag,
                    lo as usize,
                    n8,
                    s..e,
                    (frame_exp as i32) << 1,
                    inv_width,
                    nrg_est,
                    nrg_est_e,
                )
            };
            if n8 < n_bands || (high_subband.min(64) - lo) as usize > n_bands {
                calc_nrg_per_subband_straight(
                    analys_buffer_real,
                    analys_buffer_imag,
                    lo + n8 as i32,
                    high_subband,
                    start_pos,
                    next_pos,
                    frame_exp,
                    &mut nrg_est[n8..],
                    &mut nrg_est_e[n8..],
                );
            }
            return;
        }
    }
    calc_nrg_per_subband_straight(
        analys_buffer_real,
        analys_buffer_imag,
        low_subband,
        high_subband,
        start_pos,
        next_pos,
        frame_exp,
        nrg_est,
        nrg_est_e,
    );
}

/// The per-subband energy (the end of the band loop of `calcNrgPerSubband()`): `max_val` is the
/// maximum magnitude, `sum(pre_shift)` the sum of the squares (`accu`) for a pre-shift.
#[inline(always)]
fn nrg_of_subband(
    max_val: FixpDbl,
    sum: impl FnOnce(i8) -> FixpDbl,
    frame_exp: i32,
    inv_width: FixpSgl,
    lp: bool,
) -> (FixpDbl, i8) {
    if max_val == 0 {
        return (0, 0);
    }
    let pre_shift = f_min((cnt_leading_zeros(max_val) - 1) as i8 - SHIFT_BEFORE_SQUARE as i8, 25);
    let accu = sum(pre_shift) << 1;
    let mut shift = f_norm(accu) as i8;
    let nrg = f_mult(accu << shift as i32, inv_width);
    shift = (shift as i32 + 2 * pre_shift as i32) as i8;
    (nrg, (frame_exp - shift as i32 + lp as i32) as i8)
}

/// NEON kernel of [`calc_nrg_per_subband`].
#[cfg(target_arch = "aarch64")]
mod neon_nrg {
    use super::*;
    use core::arch::aarch64::*;

    /// Subbands `lo..lo + n4` (`n4` a multiple of 4) of the slots `slots`; every row covers
    /// them (checked by the caller).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn per_subband(
        re: &[&mut [FixpDbl]],
        im: Option<&[&mut [FixpDbl]]>,
        lo: usize,
        n4: usize,
        slots: core::ops::Range<usize>,
        frame_exp: i32,
        inv_width: FixpSgl,
        nrg_est: &mut [FixpDbl],
        nrg_est_e: &mut [i8],
    ) {
        let rows_re = &re[slots.clone()];
        let rows_im = im.map(|im| &im[slots.clone()]);
        for k in (0..n4).step_by(4) {
            let b = lo + k;
            assert!(rows_re.iter().all(|r| r.len() >= b + 4));
            assert!(rows_im.is_none_or(|rows| rows.iter().all(|r| r.len() >= b + 4)));
            // SAFETY: NEON is part of aarch64; every row has at least b + 4 values (above).
            unsafe {
                let ld = |r: &&mut [FixpDbl]| vld1q_s32(r.as_ptr().add(b));
                let mag = |v: int32x4_t| veorq_s32(v, vshrq_n_s32::<31>(v));
                let mut mx = vdupq_n_s32(0);
                for r in rows_re {
                    mx = vorrq_s32(mx, mag(ld(r)));
                }
                if let Some(rows) = rows_im {
                    for r in rows {
                        mx = vorrq_s32(mx, mag(ld(r)));
                    }
                }
                let mut mxa = [0i32; 4];
                vst1q_s32(mxa.as_mut_ptr(), mx);
                let ps = mxa.map(|m| {
                    if m == 0 {
                        0
                    } else {
                        f_min((cnt_leading_zeros(m) - 1) as i8 - SHIFT_BEFORE_SQUARE as i8, 25)
                            as i32
                    }
                });
                let sh = vld1q_s32(ps.as_ptr());
                let sq = |v: int32x4_t| {
                    let y = vshlq_s32(v, sh);
                    let lo = vreinterpretq_s32_s64(vmull_s32(vget_low_s32(y), vget_low_s32(y)));
                    let hi = vreinterpretq_s32_s64(vmull_high_s32(y, y));
                    vuzp2q_s32(lo, hi)
                };
                let mut acc = vdupq_n_s32(0);
                for r in rows_re {
                    acc = vaddq_s32(acc, sq(ld(r)));
                }
                if let Some(rows) = rows_im {
                    for r in rows {
                        acc = vaddq_s32(acc, sq(ld(r)));
                    }
                }
                let mut acca = [0i32; 4];
                vst1q_s32(acca.as_mut_ptr(), acc);
                for j in 0..4 {
                    let (nrg, e) =
                        nrg_of_subband(mxa[j], |_| acca[j], frame_exp, inv_width, im.is_none());
                    nrg_est[k + j] = nrg;
                    nrg_est_e[k + j] = e;
                }
            }
        }
    }
}

/// `calcNrgPerSfb()`: the mean energy of each scale factor band for the duration of the current
/// envelope (used when interpolFreq is false).
#[allow(clippy::too_many_arguments)]
pub fn calc_nrg_per_sfb(
    analys_buffer_real: &[&mut [FixpDbl]],
    analys_buffer_imag: Option<&[&mut [FixpDbl]]>,
    n_sfb: i32,
    freq_band_table: &[u8],
    start_pos: i32,
    next_pos: i32,
    input_e: i8,
    nrg_est: &mut [FixpDbl],
    nrg_est_e: &mut [i8],
) {
    // Divide by width of envelope later:
    let inv_width = fx_dbl2fx_sgl(get_inv_int(next_pos - start_pos));
    // The common exponent needs to be doubled because all mantissas are squared:
    let input_e = (input_e as i32) << 1;
    let (s, e) = (start_pos.max(0) as usize, next_pos.max(0) as usize);

    let mut out = 0usize;
    for j in 0..n_sfb.max(0) as usize {
        let li = freq_band_table.get(j).copied().unwrap_or(0) as i32;
        let ui = freq_band_table.get(j + 1).copied().unwrap_or(0) as i32;

        let max_val =
            max_subband_sample(analys_buffer_real, analys_buffer_imag, li, ui, start_pos, next_pos);

        let sum: FixpDbl;
        let sum_e: i8;
        if max_val != 0 {
            let mut pre_shift = (cnt_leading_zeros(max_val) - 1) as i8;

            // If the accu does not provide enough overflow bits, we cannot shift the samples up
            // to the limit. Instead, keep up to 3 free bits in each sample, i.e. up to 6 bits
            // after calculation of square.
            pre_shift -= SHIFT_BEFORE_SQUARE as i8;

            let mut sum_all: FixpDbl = 0;

            for k in li.max(0)..ui.min(64) {
                let ku = k as usize;
                let mut sum_line: FixpDbl = 0;

                if let Some(im) = analys_buffer_imag {
                    if pre_shift >= 0 {
                        for l in s..e {
                            let temp = analys_buffer_real[l][ku] << pre_shift as i32;
                            sum_line = sum_line.wrapping_add(f_pow2_div2(temp));
                            let temp = im[l][ku] << pre_shift as i32;
                            sum_line = sum_line.wrapping_add(f_pow2_div2(temp));
                        }
                    } else {
                        for l in s..e {
                            let temp = analys_buffer_real[l][ku] >> -(pre_shift as i32);
                            sum_line = sum_line.wrapping_add(f_pow2_div2(temp));
                            let temp = im[l][ku] >> -(pre_shift as i32);
                            sum_line = sum_line.wrapping_add(f_pow2_div2(temp));
                        }
                    }
                } else if pre_shift >= 0 {
                    for l in s..e {
                        let temp = analys_buffer_real[l][ku] << pre_shift as i32;
                        sum_line = sum_line.wrapping_add(f_pow2_div2(temp));
                    }
                } else {
                    for l in s..e {
                        let temp = analys_buffer_real[l][ku] >> -(pre_shift as i32);
                        sum_line = sum_line.wrapping_add(f_pow2_div2(temp));
                    }
                }

                // The number of QMF-channels per SBR bands may be up to 15. Shift right to avoid
                // overflows in sum over all channels.
                sum_line >>= 4 - 1;
                sum_all = sum_all.wrapping_add(sum_line);
            }

            // Convert double precision to Mantissa/Exponent:
            let shift = f_norm(sum_all) as i8;
            let mut s2 = sum_all << shift as i32;

            // Divide by width of envelope:
            s2 = f_mult(s2, inv_width);

            // Divide by width of Sfb:
            s2 = f_mult(s2, fx_dbl2fx_sgl(get_inv_int(ui - li)));
            sum = s2;

            // Set all Subband energies in the Sfb to the average energy:
            let mut se = if analys_buffer_imag.is_some() {
                input_e + 4 - shift as i32 // -4 to compensate right-shift
            } else {
                input_e + 4 + 1 - shift as i32 // -4 to compensate right-shift; +1 due to missing imag. part
            } as i8;

            se = (se as i32 - 2 * pre_shift as i32) as i8;
            sum_e = se;
        } else {
            // Prevent a zero-mantissa-number from being misinterpreted due to its exponent.
            sum = 0;
            sum_e = 0;
        }

        for _ in li..ui {
            if out < nrg_est.len() {
                nrg_est[out] = sum;
                nrg_est_e[out] = sum_e;
            }
            out += 1;
        }
    }
}

/// `calcSubbandGain()`: calculates the gain, noise and additional sine level of one subband.
#[allow(clippy::too_many_arguments)]
pub fn calc_subband_gain(
    nrg_ref: FixpDbl,
    nrg_ref_e: i8,
    nrgs: &mut EnvCalcNrgs,
    i: usize,
    tmp_noise: FixpDbl,
    tmp_noise_e: i8,
    sine_present_flag: u8,
    sine_mapped: u8,
    no_noise_flag: i32,
) {
    let mut nrg_est = nrgs.nrg_est[i];
    let mut nrg_est_e = nrgs.nrg_est_e[i];

    // This addition of 1 prevents divisions by zero in the reference code. For very small
    // energies in nrgEst, it prevents the gains from becoming very high which could cause some
    // trouble due to the smoothing.
    // `SCHAR b_e = (int)(nrgEst_e - 1)`: the difference is truncated to 8 bits
    let mut b_e = (nrg_est_e as i32 - 1) as i8 as i32;
    if b_e >= 0 {
        nrg_est = (fl2fxconst_dbl(0.5f32 as f64) >> fix_min(b_e + 1, DFRACT_BITS - 1))
            .wrapping_add(nrg_est >> 1);
        nrg_est_e = nrg_est_e.wrapping_add(1); // shift by 1 bit to avoid overflow
    } else {
        nrg_est = (nrg_est >> fix_min(-b_e + 1, DFRACT_BITS - 1))
            .wrapping_add(fl2fxconst_dbl(0.5f32 as f64) >> 1);
        nrg_est_e = 2; // shift by 1 bit to avoid overflow
    }

    //  A = NrgRef * TmpNoise
    let a = f_mult(nrg_ref, tmp_noise);
    let a_e = (nrg_ref_e as i32 + tmp_noise_e as i32) as i8;

    //  B = 1 + TmpNoise
    b_e = (tmp_noise_e as i32 - 1) as i8 as i32; // SCHAR
    let mut b: FixpDbl;
    let mut b_e8: i8;
    if b_e >= 0 {
        b = (fl2fxconst_dbl(0.5f32 as f64) >> fix_min(b_e + 1, DFRACT_BITS - 1))
            .wrapping_add(tmp_noise >> 1);
        b_e8 = (tmp_noise_e as i32 + 1) as i8; // shift by 1 bit to avoid overflow
    } else {
        b = (tmp_noise >> fix_min(-b_e + 1, DFRACT_BITS - 1))
            .wrapping_add(fl2fxconst_dbl(0.5f32 as f64) >> 1);
        b_e8 = 2; // shift by 1 bit to avoid overflow
    }

    //  noiseLevel = A / B = (NrgRef * TmpNoise) / (1 + TmpNoise)
    (nrgs.noise_level[i], nrgs.noise_level_e[i]) = fdk_divide_mant_exp(a, a_e, b, b_e8);

    if sine_present_flag != 0 {
        //  C = (1 + TmpNoise) * NrgEst
        let c = f_mult(b, nrg_est);
        let c_e = (b_e8 as i32 + nrg_est_e as i32) as i8;

        //  gain = A / C = (NrgRef * TmpNoise) / (1 + TmpNoise) * NrgEst
        (nrgs.nrg_gain[i], nrgs.nrg_gain_e[i]) = fdk_divide_mant_exp(a, a_e, c, c_e);

        if sine_mapped != 0 {
            //  sineLevel = nrgRef/ (1 + TmpNoise)
            (nrgs.nrg_sine[i], nrgs.nrg_sine_e[i]) =
                fdk_divide_mant_exp(nrg_ref, nrg_ref_e, b, b_e8);
        }
    } else {
        if no_noise_flag != 0 {
            //  B = NrgEst
            b = nrg_est;
            b_e8 = nrg_est_e;
        } else {
            //  B = NrgEst * (1 + TmpNoise)
            b = f_mult(b, nrg_est);
            b_e8 = (b_e8 as i32 + nrg_est_e as i32) as i8;
        }

        //  gain = nrgRef / B
        let mut result_exp = 0i32;
        nrgs.nrg_gain[i] = f_div_norm_fast(nrg_ref, b, &mut result_exp);
        nrgs.nrg_gain_e[i] = ((result_exp as i8 as i32) + (nrg_ref_e as i32 - b_e8 as i32)) as i8;

        // There could be a one bit diffs. This is important to compensate, because later in the
        // code values are compared by exponent only.
        let headroom = count_leading_bits(nrgs.nrg_gain[i]);
        nrgs.nrg_gain[i] <<= headroom;
        nrgs.nrg_gain_e[i] = (nrgs.nrg_gain_e[i] as i32 - headroom) as i8;
    }
}

/// `calcAvgGain()`: the "average gain" of a limiter band (rather a gain of the average
/// magnitude than the average of gains): `(sumRef, sumRef_e, avgGain, avgGain_e)`.
pub fn calc_avg_gain(
    nrgs: &EnvCalcNrgs,
    low_subband: usize,
    high_subband: usize,
) -> (FixpDbl, i8, FixpDbl, i8) {
    let mut sum_ref: FixpDbl = 1;
    let mut sum_est: FixpDbl = 1;
    let mut sum_ref_e: i8 = -FRACT_BITS as i8;
    let mut sum_est_e: i8 = -FRACT_BITS as i8;

    for k in low_subband..high_subband.min(MAX_FREQ_COEFFS) {
        // Add nrgRef[k] to sumRef:
        (sum_ref, sum_ref_e) =
            fdk_add_mant_exp(sum_ref, sum_ref_e, nrgs.nrg_ref[k], nrgs.nrg_ref_e[k]);

        // Add nrgEst[k] to sumEst:
        (sum_est, sum_est_e) =
            fdk_add_mant_exp(sum_est, sum_est_e, nrgs.nrg_est[k], nrgs.nrg_est_e[k]);
    }

    let (avg, avg_e) = fdk_divide_mant_exp(sum_ref, sum_ref_e, sum_est, sum_est_e);

    (sum_ref, sum_ref_e, avg, avg_e)
}

/// `adjustTimeSlot_EldGrid()`: amplifies one time slot of the real valued (low power) signal of
/// the low delay grid with the calculated gains and adds noise and sines. `slot` is the whole
/// slot (`ptrReal` is `&slot[lowSubband]`, `ptrReal - 1` is read and written).
#[allow(clippy::too_many_arguments)]
///
/// The straight port, kept as the reference of the optimized [`adjust_time_slot_eld_grid`].
pub fn adjust_time_slot_eld_grid_straight(
    slot: &mut [FixpDbl],
    low_subband: i32,
    nrgs: &EnvCalcNrgs,
    ptr_harm_index: &mut u8,
    no_subbands: i32,
    scale_change: i32,
    no_noise_flag: i32,
    ptr_phase_index: &mut i32,
    scale_diff_low: i32,
) {
    let mut tone_count = 0;

    let p_gain = &nrgs.nrg_gain; // Gains of current envelope
    let p_noise_level = &nrgs.noise_level; // Noise levels of current envelope
    let p_sine_level = &nrgs.nrg_sine; // Sine levels

    let mut phase_index = *ptr_phase_index;
    let harm_index = *ptr_harm_index as usize & 3;

    let p_harmonic_phase_x = &HARMONIC_PHASE_X[harm_index];
    let p_harmonic_phase = &HARMONIC_PHASE[harm_index];

    let max_val = MAX_VAL_NRG_HEADROOM.wrapping_shr(scale_change as u32); // (register shift)
    let min_val = -max_val;

    let lo = low_subband as usize;
    let at = |i: usize| p_sine_level.get(i).copied().unwrap_or(0);

    // *(ptrReal - 1) = fAddSaturate(*(ptrReal - 1), SATURATE_SHIFT(fMultDiv2(
    //     p_harmonicPhaseX[lowSubband & 1], pSineLevel[0]), scale_diff_low, DFRACT_BITS))
    if lo >= 1 {
        let v = saturate_shift(
            f_mult_div2(p_harmonic_phase_x[(low_subband & 1) as usize], at(0)),
            scale_diff_low,
            DFRACT_BITS,
        );
        slot[lo - 1] = f_add_saturate(slot[lo - 1], v);
    }
    let mut p_sine_level_prev: FixpDbl = 0;

    let mut idx_k = (low_subband & 1) as usize;

    let ns = no_subbands.max(0) as usize;
    let mut k = 0usize;
    while k < ns {
        let sine_level_curr = at(k);
        phase_index = (phase_index + 1) & (SBR_NF_NO_RANDOM_VAL - 1);

        let p = lo + k;
        let mut signal_real = f_max(f_min(f_mult_div2(slot[p], p_gain[k]), max_val), min_val)
            .wrapping_shl(scale_change as u32);
        let sb_noise = p_noise_level[k];
        if (sine_level_curr | no_noise_flag) == 0 {
            signal_real = signal_real.wrapping_add(f_mult(
                FDK_SBR_DECODER_SBR_RANDOM_PHASE[phase_index as usize][0],
                sb_noise,
            ));
        }
        signal_real = signal_real.wrapping_add(sine_level_curr.wrapping_mul(p_harmonic_phase[0]));
        signal_real = f_mult_add_div2(signal_real, p_sine_level_prev, p_harmonic_phase_x[idx_k]);
        p_sine_level_prev = sine_level_curr;
        idx_k ^= 1;
        if (k as i32) < no_subbands - 1 {
            signal_real = f_mult_add_div2(signal_real, at(k + 1), p_harmonic_phase_x[idx_k]);
        } else {
            // (k == noSubbands - 1)
            if (k as i32) + low_subband + 1 < 63 {
                slot[p + 1] = slot[p + 1]
                    .wrapping_add(f_mult_div2(p_sine_level_prev, p_harmonic_phase_x[idx_k]));
            }
        }
        slot[p] = signal_real;

        if p_sine_level_prev != 0 {
            tone_count += 1;
            if tone_count == 16 {
                k += 1;
                break;
            }
        }
        k += 1;
    }
    // Run again, if previous loop got breaked with tone_count = 16
    while k < ns {
        let sine_level_curr = at(k);
        phase_index = (phase_index + 1) & (SBR_NF_NO_RANDOM_VAL - 1);

        let p = lo + k;
        let mut signal_real = f_max(f_min(f_mult_div2(slot[p], p_gain[k]), max_val), min_val)
            .wrapping_shl(scale_change as u32);
        let sb_noise = p_noise_level[k];
        if (sine_level_curr | no_noise_flag) == 0 {
            signal_real = signal_real.wrapping_add(f_mult(
                FDK_SBR_DECODER_SBR_RANDOM_PHASE[phase_index as usize][0],
                sb_noise,
            ));
        }
        signal_real = signal_real.wrapping_add(sine_level_curr.wrapping_mul(p_harmonic_phase[0]));
        slot[p] = signal_real;
        k += 1;
    }

    *ptr_harm_index = ((harm_index + 1) & 3) as u8;
    *ptr_phase_index = phase_index & (SBR_NF_NO_RANDOM_VAL - 1);
}

/// `C1`: `FL2FXCONST_SGL(2.f * 0.00815f)`
const C1: FixpSgl = fl2fxconst_sgl((2.0f32 * 0.00815f32) as f64);

/// `adjustTimeSlotLC()`: amplifies one time slot of the real valued (low power) signal with the
/// calculated gains and adds the noise floor and sines. `slot` is the whole slot (`ptrReal` is
/// `&slot[lowSubband]`).
///
/// The final block always processes the subband after the middle loop: for `noSubbands == 1`
/// that is subband 1 (the C code does the same).
#[allow(clippy::too_many_arguments)]
///
/// The straight port, kept as the reference of the optimized [`adjust_time_slot_lc`].
pub fn adjust_time_slot_lc_straight(
    slot: &mut [FixpDbl],
    low_subband: i32,
    nrgs: &EnvCalcNrgs,
    ptr_harm_index: &mut u8,
    no_subbands: i32,
    scale_change: i32,
    no_noise_flag: i32,
    ptr_phase_index: &mut i32,
) {
    let p_gain = &nrgs.nrg_gain; // Gains of current envelope
    let p_noise_level = &nrgs.noise_level; // Noise levels of current envelope
    let p_sine = &nrgs.nrg_sine; // Sine levels
    let sine = |i: isize| -> FixpDbl {
        if i < 0 {
            0
        } else {
            p_sine.get(i as usize).copied().unwrap_or(0)
        }
    };
    let gain = |i: usize| p_gain.get(i).copied().unwrap_or(0);
    let noise = |i: usize| p_noise_level.get(i).copied().unwrap_or(0);

    let mut index = *ptr_phase_index;
    let harm_index = *ptr_harm_index;
    let mut freq_inv_flag = (low_subband & 1) != 0;
    let mut tone_count = 0;
    let mut sine_sign = true;
    let max_val = MAX_VAL_NRG_HEADROOM.wrapping_shr(scale_change as u32); // (register shift)
    let min_val = -max_val;

    let lo = low_subband as usize;
    let mut p = lo; // ptrReal
    let mut g = 0usize; // pGain
    let mut sl = 0isize; // pSineLevel
    let mut nl = 0usize; // pNoiseLevel
    let get = |slot: &[FixpDbl], p: usize| slot.get(p).copied().unwrap_or(0);

    // First pass for k=0 pulled out of the loop:

    index = (index + 1) & (SBR_NF_NO_RANDOM_VAL - 1);

    // The next multiplication constitutes the actual envelope adjustment of the signal and
    // should be carried out with full accuracy (supplying #FRACT_BITS valid bits).
    let mut signal_real = f_max(f_min(f_mult_div2(get(slot, p), gain(g)), max_val), min_val)
        .wrapping_shl(scale_change as u32);
    g += 1;
    let mut sine_level = sine(sl);
    sl += 1;
    let sine_level_next = if no_subbands > 1 { sine(sl) } else { 0 };

    if sine_level != 0 {
        tone_count += 1;
    } else if no_noise_flag == 0 {
        // Add noisefloor to the amplified signal
        signal_real = signal_real
            .wrapping_add(f_mult(FDK_SBR_DECODER_SBR_RANDOM_PHASE[index as usize][0], noise(nl)));
    }

    {
        if harm_index & 0x1 == 0 {
            // harmIndex 0,2
            signal_real = signal_real.wrapping_add(if harm_index & 0x2 != 0 {
                sine_level.wrapping_neg()
            } else {
                sine_level
            });
            if let Some(v) = slot.get_mut(p) {
                *v = signal_real;
            }
            p += 1;
        } else {
            // harmIndex 1,3 in combination with freqInvFlag
            let mut shift = scale_change + 1;
            shift = if shift >= 0 {
                fix_min(DFRACT_BITS - 1, shift)
            } else {
                fix_max(-(DFRACT_BITS - 1), shift)
            };

            let tmp1 = if shift >= 0 {
                f_mult_div2(C1, sine_level) >> shift
            } else {
                f_mult_div2(C1, sine_level) << (-shift)
            };
            let tmp2 = f_mult_div2(C1, sine_level_next);

            // save switch and compare operations and reduce to XOR statement
            if (((harm_index >> 1) & 0x1) != 0) ^ freq_inv_flag {
                if p >= 1 {
                    slot[p - 1] = f_add_saturate(slot[p - 1], tmp1);
                }
                signal_real = signal_real.wrapping_sub(tmp2);
            } else {
                if p >= 1 {
                    slot[p - 1] = f_add_saturate(slot[p - 1], tmp1.wrapping_neg());
                }
                signal_real = signal_real.wrapping_add(tmp2);
            }
            if let Some(v) = slot.get_mut(p) {
                *v = signal_real;
            }
            p += 1;
            freq_inv_flag = !freq_inv_flag;
        }
    }

    nl += 1;

    if no_subbands > 2 {
        if harm_index & 0x1 == 0 {
            // harmIndex 0,2
            if harm_index == 0 {
                sine_sign = false;
            }

            let mut k = no_subbands - 2;
            while k != 0 {
                let sinelevel = sine(sl);
                sl += 1;
                index += 1;
                signal_real = if sine_sign { sinelevel.wrapping_neg() } else { sinelevel };
                if signal_real == 0 && no_noise_flag == 0 {
                    // Add noisefloor to the amplified signal
                    index &= SBR_NF_NO_RANDOM_VAL - 1;
                    signal_real = signal_real.wrapping_add(f_mult(
                        FDK_SBR_DECODER_SBR_RANDOM_PHASE[index as usize][0],
                        noise(nl),
                    ));
                }

                // The next multiplication constitutes the actual envelope adjustment of the
                // signal.
                signal_real = signal_real.wrapping_add(
                    f_max(f_min(f_mult_div2(get(slot, p), gain(g)), max_val), min_val)
                        .wrapping_shl(scale_change as u32),
                );
                g += 1;

                nl += 1;
                if let Some(v) = slot.get_mut(p) {
                    *v = signal_real;
                }
                p += 1;
                k -= 1;
            }
        } else {
            // harmIndex 1,3 in combination with freqInvFlag
            if harm_index == 1 {
                freq_inv_flag = !freq_inv_flag;
            }

            let mut k = no_subbands - 2;
            while k != 0 {
                index += 1;
                // The next multiplication constitutes the actual envelope adjustment of the
                // signal.
                signal_real = f_max(f_min(f_mult_div2(get(slot, p), gain(g)), max_val), min_val)
                    .wrapping_shl(scale_change as u32);
                g += 1;

                let s = sine(sl);
                sl += 1;
                if s != 0 {
                    tone_count += 1;
                } else if no_noise_flag == 0 {
                    // Add noisefloor to the amplified signal
                    index &= SBR_NF_NO_RANDOM_VAL - 1;
                    signal_real = signal_real.wrapping_add(f_mult(
                        FDK_SBR_DECODER_SBR_RANDOM_PHASE[index as usize][0],
                        noise(nl),
                    ));
                }

                nl += 1;

                if tone_count <= 16 {
                    let add_sine = f_mult_div2(sine(sl - 2).wrapping_sub(sine(sl)), C1);
                    signal_real = signal_real.wrapping_add(if freq_inv_flag {
                        add_sine.wrapping_neg()
                    } else {
                        add_sine
                    });
                }

                if let Some(v) = slot.get_mut(p) {
                    *v = signal_real;
                }
                p += 1;
                freq_inv_flag = !freq_inv_flag;
                k -= 1;
            }
        }
    }

    if no_subbands > -1 {
        index += 1;
        // The next multiplication constitutes the actual envelope adjustment of the signal.
        signal_real = f_max(f_min(f_mult_div2(get(slot, p), gain(g)), max_val), min_val)
            .wrapping_shl(scale_change as u32);
        let sine_level_prev = f_mult_div2(sine(sl - 1), fl2fx_sgl(0.0163f32));
        sine_level = sine(sl);

        if sine(sl) != 0 {
            tone_count += 1;
        } else if no_noise_flag == 0 {
            // Add noisefloor to the amplified signal
            index &= SBR_NF_NO_RANDOM_VAL - 1;
            signal_real = signal_real.wrapping_add(f_mult(
                FDK_SBR_DECODER_SBR_RANDOM_PHASE[index as usize][0],
                noise(nl),
            ));
        }

        if harm_index & 0x1 == 0 {
            // harmIndex 0,2
            if let Some(v) = slot.get_mut(p) {
                *v = signal_real.wrapping_add(if sine_sign {
                    sine_level.wrapping_neg()
                } else {
                    sine_level
                });
            }
        } else {
            // harmIndex 1,3 in combination with freqInvFlag
            if tone_count <= 16 {
                if freq_inv_flag {
                    if let Some(v) = slot.get_mut(p) {
                        *v = signal_real.wrapping_sub(sine_level_prev);
                    }
                    p += 1;
                    if no_subbands + low_subband < 63 {
                        if let Some(v) = slot.get_mut(p) {
                            *v = v.wrapping_add(f_mult_div2(C1, sine_level));
                        }
                    }
                } else {
                    if let Some(v) = slot.get_mut(p) {
                        *v = signal_real.wrapping_add(sine_level_prev);
                    }
                    p += 1;
                    if no_subbands + low_subband < 63 {
                        if let Some(v) = slot.get_mut(p) {
                            *v = v.wrapping_sub(f_mult_div2(C1, sine_level));
                        }
                    }
                }
            } else if let Some(v) = slot.get_mut(p) {
                *v = signal_real;
            }
        }
    }
    *ptr_harm_index = (harm_index.wrapping_add(1)) & 3;
    *ptr_phase_index = index & (SBR_NF_NO_RANDOM_VAL - 1);
}

/// `adjustTimeSlotHQ_GainAndNoise()`: variant of `adjustTimeSlotHQ()` which only regards gain
/// and noise but no additional harmonics. `ptr_real`/`ptr_imag` start at `lowSubband`.
#[allow(clippy::too_many_arguments)]
///
/// The straight port, kept as the reference of the optimized [`adjust_time_slot_hq_gain_and_noise`].
pub fn adjust_time_slot_hq_gain_and_noise_straight(
    ptr_real: &mut [FixpDbl],
    ptr_imag: &mut [FixpDbl],
    h_sbr_cal_env: &mut SbrCalculateEnvelope,
    nrgs: &EnvCalcNrgs,
    _low_subband: i32,
    no_subbands: i32,
    scale_change: i32,
    smooth_ratio: FixpSgl,
    no_noise_flag: i32,
    mut filt_buffer_noise_shift: i32,
) {
    let gain = &nrgs.nrg_gain; // Gains of current envelope
    let noise_level = &nrgs.noise_level; // Noise levels of current envelope
    let p_sine_level = &nrgs.nrg_sine; // Sine levels

    let direct_ratio: FixpSgl = (MAXVAL_SGL as FixpSgl).wrapping_sub(smooth_ratio);
    let mut index = h_sbr_cal_env.phase_index;
    let shift: i32;
    let mut max_val_noise: FixpDbl = 0;
    let mut min_val_noise: FixpDbl = 0;
    let max_val = MAX_VAL_NRG_HEADROOM.wrapping_shr(scale_change as u32); // (register shift)
    let min_val = -max_val;

    h_sbr_cal_env.phase_index = (index + no_subbands) & (SBR_NF_NO_RANDOM_VAL - 1);

    filt_buffer_noise_shift += 1; // due to later use of fMultDiv2 instead of fMult
    if filt_buffer_noise_shift < 0 {
        shift = fix_min(DFRACT_BITS - 1, -filt_buffer_noise_shift);
    } else {
        shift = fix_min(DFRACT_BITS - 1, filt_buffer_noise_shift);
        max_val_noise = MAX_VAL_NRG_HEADROOM >> shift;
        min_val_noise = -max_val_noise;
    }

    let n = (no_subbands.max(0) as usize).min(ptr_real.len()).min(ptr_imag.len());
    let filt_buffer = &h_sbr_cal_env.filt_buffer;
    let filt_buffer_noise = &h_sbr_cal_env.filt_buffer_noise;

    if smooth_ratio > 0 {
        for k in 0..n {
            // Smoothing: The old envelope has been bufferd and a certain ratio of the old gains
            // and noise levels is used.
            let smoothed_gain =
                f_mult(smooth_ratio, filt_buffer[k]).wrapping_add(f_mult(direct_ratio, gain[k]));

            let mut smoothed_noise = if filt_buffer_noise_shift < 0 {
                (f_mult_div2(smooth_ratio, filt_buffer_noise[k]) >> shift)
                    .wrapping_add(f_mult(direct_ratio, noise_level[k]))
            } else {
                let sn = f_mult_div2(smooth_ratio, filt_buffer_noise[k]);
                (f_max(f_min(sn, max_val_noise), min_val_noise) << shift)
                    .wrapping_add(f_mult(direct_ratio, noise_level[k]))
            };

            smoothed_noise = f_max(f_min(smoothed_noise, MAXVAL_DBL / 2), MINVAL_DBL / 2);

            // The next 2 multiplications constitute the actual envelope adjustment of the signal
            // and should be carried out with full accuracy (supplying #DFRACT_BITS valid bits).
            let signal_real =
                f_max(f_min(f_mult_div2(ptr_real[k], smoothed_gain), max_val), min_val)
                    .wrapping_shl(scale_change as u32);
            let signal_imag =
                f_max(f_min(f_mult_div2(ptr_imag[k], smoothed_gain), max_val), min_val)
                    .wrapping_shl(scale_change as u32);

            index += 1;

            if p_sine_level[k] != 0 || no_noise_flag != 0 {
                // Just the amplified signal is saved
                ptr_real[k] = signal_real;
                ptr_imag[k] = signal_imag;
            } else {
                // Add noisefloor to the amplified signal
                index &= SBR_NF_NO_RANDOM_VAL - 1;
                let noise_real =
                    f_mult(FDK_SBR_DECODER_SBR_RANDOM_PHASE[index as usize][0], smoothed_noise);
                let noise_imag =
                    f_mult(FDK_SBR_DECODER_SBR_RANDOM_PHASE[index as usize][1], smoothed_noise);
                ptr_real[k] = signal_real.wrapping_add(noise_real);
                ptr_imag[k] = signal_imag.wrapping_add(noise_imag);
            }
        }
    } else {
        for k in 0..n {
            let smoothed_gain = gain[k];
            let mut signal_real =
                f_max(f_min(f_mult_div2(ptr_real[k], smoothed_gain), max_val), min_val)
                    .wrapping_shl(scale_change as u32);
            let mut signal_imag =
                f_max(f_min(f_mult_div2(ptr_imag[k], smoothed_gain), max_val), min_val)
                    .wrapping_shl(scale_change as u32);

            index += 1;

            if p_sine_level[k] == 0 && no_noise_flag == 0 {
                // Add noisefloor to the amplified signal
                let smoothed_noise = noise_level[k];
                index &= SBR_NF_NO_RANDOM_VAL - 1;
                let noise_real =
                    f_mult(FDK_SBR_DECODER_SBR_RANDOM_PHASE[index as usize][0], smoothed_noise);
                let noise_imag =
                    f_mult(FDK_SBR_DECODER_SBR_RANDOM_PHASE[index as usize][1], smoothed_noise);

                signal_real = signal_real.wrapping_add(noise_real);
                signal_imag = signal_imag.wrapping_add(noise_imag);
            }
            ptr_real[k] = signal_real;
            ptr_imag[k] = signal_imag;
        }
    }
}

/// `adjustTimeSlotHQ_AddHarmonics()`: variant of `adjustTimeSlotHQ()` which only adds the
/// additional harmonics. `ptr_real`/`ptr_imag` start at `lowSubband`.
pub fn adjust_time_slot_hq_add_harmonics(
    ptr_real: &mut [FixpDbl],
    ptr_imag: &mut [FixpDbl],
    h_sbr_cal_env: &mut SbrCalculateEnvelope,
    nrgs: &EnvCalcNrgs,
    low_subband: i32,
    no_subbands: i32,
    scale_change: i32,
) {
    let p_sine_level = &nrgs.nrg_sine; // Sine levels
    let harm_index = h_sbr_cal_env.harm_index;
    let mut freq_inv_flag = low_subband & 1;

    h_sbr_cal_env.harm_index = harm_index.wrapping_add(1) & 3;

    let n = (no_subbands.max(0) as usize).min(ptr_real.len()).min(ptr_imag.len());
    for k in 0..n {
        let mut sine_level = p_sine_level[k];
        freq_inv_flag ^= 1;
        if sine_level != 0 {
            let signal_real = ptr_real[k];
            let signal_imag = ptr_imag[k];
            sine_level = scale_value(sine_level, scale_change);
            if harm_index & 2 != 0 {
                // case 2,3
                sine_level = sine_level.wrapping_neg();
            }
            if harm_index & 1 == 0 {
                // case 0,2:
                ptr_real[k] = signal_real.wrapping_add(sine_level);
            } else {
                // case 1,3
                if freq_inv_flag == 0 {
                    sine_level = sine_level.wrapping_neg();
                }
                ptr_imag[k] = signal_imag.wrapping_add(sine_level);
            }
        }
    }
}

/// `adjustTimeSlotHQ()`: amplifies one time slot of the complex signal with the calculated gains
/// (with time smoothing) and adds the noise floor and sines. `ptr_real`/`ptr_imag` start at
/// `lowSubband`.
#[allow(clippy::too_many_arguments)]
///
/// The straight port, kept as the reference of the optimized [`adjust_time_slot_hq`].
pub fn adjust_time_slot_hq_straight(
    ptr_real: &mut [FixpDbl],
    ptr_imag: &mut [FixpDbl],
    h_sbr_cal_env: &mut SbrCalculateEnvelope,
    nrgs: &EnvCalcNrgs,
    low_subband: i32,
    no_subbands: i32,
    scale_change: i32,
    smooth_ratio: FixpSgl,
    no_noise_flag: i32,
    mut filt_buffer_noise_shift: i32,
) {
    let gain = &nrgs.nrg_gain; // Gains of current envelope
    let noise_level = &nrgs.noise_level; // Noise levels of current envelope
    let p_sine_level = &nrgs.nrg_sine; // Sine levels

    let direct_ratio: FixpSgl = (MAXVAL_SGL as FixpSgl).wrapping_sub(smooth_ratio);
    let mut index = h_sbr_cal_env.phase_index;
    let harm_index = h_sbr_cal_env.harm_index;
    let mut freq_inv_flag = low_subband & 1;
    let shift: i32;
    let mut max_val_noise: FixpDbl = 0;
    let mut min_val_noise: FixpDbl = 0;
    let max_val = MAX_VAL_NRG_HEADROOM.wrapping_shr(scale_change as u32); // (register shift)
    let min_val = -max_val;

    h_sbr_cal_env.phase_index = (index + no_subbands) & (SBR_NF_NO_RANDOM_VAL - 1);
    h_sbr_cal_env.harm_index = harm_index.wrapping_add(1) & 3;

    // Possible optimization: smooth_ratio and harmIndex stay constant during the loop. It might
    // be faster to include a separate loop in each path. the check for smooth_ratio is now
    // outside the loop and the workload of the whole function decreased by about 20 %

    filt_buffer_noise_shift += 1; // due to later use of fMultDiv2 instead of fMult
    if filt_buffer_noise_shift < 0 {
        shift = fix_min(DFRACT_BITS - 1, -filt_buffer_noise_shift);
    } else {
        shift = fix_min(DFRACT_BITS - 1, filt_buffer_noise_shift);
        max_val_noise = MAX_VAL_NRG_HEADROOM >> shift;
        min_val_noise = -max_val_noise;
    }

    let n = (no_subbands.max(0) as usize).min(ptr_real.len()).min(ptr_imag.len());
    let filt_buffer = &h_sbr_cal_env.filt_buffer;
    let filt_buffer_noise = &h_sbr_cal_env.filt_buffer_noise;

    if smooth_ratio > 0 {
        for k in 0..n {
            // Smoothing: The old envelope has been bufferd and a certain ratio of the old gains
            // and noise levels is used.

            let smoothed_gain =
                f_mult(smooth_ratio, filt_buffer[k]).wrapping_add(f_mult(direct_ratio, gain[k]));

            let mut smoothed_noise = if filt_buffer_noise_shift < 0 {
                (f_mult_div2(smooth_ratio, filt_buffer_noise[k]) >> shift)
                    .wrapping_add(f_mult(direct_ratio, noise_level[k]))
            } else {
                let sn = f_mult_div2(smooth_ratio, filt_buffer_noise[k]);
                (f_max(f_min(sn, max_val_noise), min_val_noise) << shift)
                    .wrapping_add(f_mult(direct_ratio, noise_level[k]))
            };

            smoothed_noise = f_max(f_min(smoothed_noise, MAXVAL_DBL / 2), MINVAL_DBL / 2);

            // The next 2 multiplications constitute the actual envelope adjustment of the signal
            // and should be carried out with full accuracy (supplying #DFRACT_BITS valid bits).
            let signal_real =
                f_max(f_min(f_mult_div2(ptr_real[k], smoothed_gain), max_val), min_val)
                    .wrapping_shl(scale_change as u32);
            let signal_imag =
                f_max(f_min(f_mult_div2(ptr_imag[k], smoothed_gain), max_val), min_val)
                    .wrapping_shl(scale_change as u32);

            index += 1;

            if p_sine_level[k] != 0 {
                let sine_level = p_sine_level[k];

                match harm_index {
                    0 => {
                        ptr_real[k] = signal_real.wrapping_add(sine_level);
                        ptr_imag[k] = signal_imag;
                    }
                    2 => {
                        ptr_real[k] = signal_real.wrapping_sub(sine_level);
                        ptr_imag[k] = signal_imag;
                    }
                    1 => {
                        ptr_real[k] = signal_real;
                        if freq_inv_flag != 0 {
                            ptr_imag[k] = signal_imag.wrapping_sub(sine_level);
                        } else {
                            ptr_imag[k] = signal_imag.wrapping_add(sine_level);
                        }
                    }
                    _ => {
                        ptr_real[k] = signal_real;
                        if freq_inv_flag != 0 {
                            ptr_imag[k] = signal_imag.wrapping_add(sine_level);
                        } else {
                            ptr_imag[k] = signal_imag.wrapping_sub(sine_level);
                        }
                    }
                }
            } else if no_noise_flag != 0 {
                // Just the amplified signal is saved
                ptr_real[k] = signal_real;
                ptr_imag[k] = signal_imag;
            } else {
                // Add noisefloor to the amplified signal
                index &= SBR_NF_NO_RANDOM_VAL - 1;
                let noise_real =
                    f_mult(FDK_SBR_DECODER_SBR_RANDOM_PHASE[index as usize][0], smoothed_noise);
                let noise_imag =
                    f_mult(FDK_SBR_DECODER_SBR_RANDOM_PHASE[index as usize][1], smoothed_noise);
                ptr_real[k] = signal_real.wrapping_add(noise_real);
                ptr_imag[k] = signal_imag.wrapping_add(noise_imag);
            }
            freq_inv_flag ^= 1;
        }
    } else {
        for k in 0..n {
            let smoothed_gain = gain[k];
            let mut signal_real =
                f_max(f_min(f_mult_div2(ptr_real[k], smoothed_gain), max_val), min_val)
                    .wrapping_shl(scale_change as u32);
            let mut signal_imag =
                f_max(f_min(f_mult_div2(ptr_imag[k], smoothed_gain), max_val), min_val)
                    .wrapping_shl(scale_change as u32);

            index += 1;

            let sine_level = p_sine_level[k];
            if sine_level != 0 {
                match harm_index {
                    0 => signal_real = signal_real.wrapping_add(sine_level),
                    1 => {
                        if freq_inv_flag != 0 {
                            signal_imag = signal_imag.wrapping_sub(sine_level);
                        } else {
                            signal_imag = signal_imag.wrapping_add(sine_level);
                        }
                    }
                    2 => signal_real = signal_real.wrapping_sub(sine_level),
                    3 => {
                        if freq_inv_flag != 0 {
                            signal_imag = signal_imag.wrapping_add(sine_level);
                        } else {
                            signal_imag = signal_imag.wrapping_sub(sine_level);
                        }
                    }
                    _ => {}
                }
            } else if no_noise_flag == 0 {
                // Add noisefloor to the amplified signal
                let smoothed_noise = noise_level[k];
                index &= SBR_NF_NO_RANDOM_VAL - 1;
                let noise_real =
                    f_mult(FDK_SBR_DECODER_SBR_RANDOM_PHASE[index as usize][0], smoothed_noise);
                let noise_imag =
                    f_mult(FDK_SBR_DECODER_SBR_RANDOM_PHASE[index as usize][1], smoothed_noise);

                signal_real = signal_real.wrapping_add(noise_real);
                signal_imag = signal_imag.wrapping_add(noise_imag);
            }
            ptr_real[k] = signal_real;
            ptr_imag[k] = signal_imag;

            freq_inv_flag ^= 1;
        }
    }
}

/// `ResetLimiterBands()`: builds the frequency band table of the gain limiter from the patch
/// borders of the transposer. `x_over_qmf` is `NULL` (`None`) without HBE.
#[allow(clippy::too_many_arguments)]
pub fn reset_limiter_bands(
    limiter_band_table: &mut [u8; MAX_NUM_LIMITERS + 1],
    no_limiter_bands: &mut u8,
    freq_band_table: &[u8],
    no_freq_bands: i32,
    patch_param: &[PatchParam],
    mut no_patches: i32,
    limiter_bands: i32,
    sbr_patching_mode: u8,
    x_over_qmf: Option<&[i32; MAX_NUM_PATCHES_HBE]>,
    b41_sbr: i32,
) -> SbrError {
    let mut work_limiter_band_table = [0u8; MAX_FREQ_COEFFS / 2 + MAX_NUM_PATCHES + 1 + 64];
    let mut patch_borders = [0i32; MAX_NUM_PATCHES + 1];
    let n_bands: i32;
    let fbt = |i: usize| freq_band_table.get(i).copied().unwrap_or(0) as i32;

    let low_subband = fbt(0);
    let high_subband = fbt(no_freq_bands.max(0) as usize);

    // 1 limiter band.
    if limiter_bands == 0 {
        limiter_band_table[0] = 0;
        limiter_band_table[1] = (high_subband - low_subband) as u8;
        n_bands = 1;
    } else {
        let mut i: usize;
        if sbr_patching_mode == 0 && x_over_qmf.is_some() {
            let x = x_over_qmf.unwrap_or(&[0; MAX_NUM_PATCHES_HBE]);
            no_patches = 0;

            if b41_sbr == 1 {
                for i in 1..MAX_NUM_PATCHES_HBE {
                    if x[i] != 0 {
                        no_patches += 1;
                    }
                }
            } else {
                for i in 1..MAX_STRETCH_HBE {
                    if x[i] != 0 {
                        no_patches += 1;
                    }
                }
            }
            i = 0;
            while i < no_patches as usize {
                patch_borders[i] = x[i] - low_subband;
                i += 1;
            }
        } else {
            // noPatches <= MAX_NUM_PATCHES (resetLppTransposer())
            no_patches = no_patches.clamp(0, MAX_NUM_PATCHES as i32);
            i = 0;
            while i < no_patches as usize {
                patch_borders[i] = patch_param[i].guard_start_band as i32 - low_subband;
                i += 1;
            }
        }
        patch_borders[i] = high_subband - low_subband;

        // 1.2, 2, or 3 limiter bands/octave plus bandborders at patchborders.
        for k in 0..=no_freq_bands.max(0) as usize {
            if let Some(w) = work_limiter_band_table.get_mut(k) {
                *w = (fbt(k) - low_subband) as u8;
            }
        }
        for k in 1..no_patches.max(0) as usize {
            if let Some(w) = work_limiter_band_table.get_mut(no_freq_bands as usize + k) {
                *w = patch_borders[k] as u8;
            }
        }

        let temp_no_lim = no_freq_bands + no_patches - 1;
        let mut nb = temp_no_lim;
        let sort_len = ((temp_no_lim + 1).clamp(0, work_limiter_band_table.len() as i32)) as u8;
        shellsort(&mut work_limiter_band_table, sort_len);

        let mut lo_lim_index = 0usize;
        let mut hi_lim_index = 1usize;

        let lbpo =
            FDK_SBR_DECODER_SBR_LIMITER_BANDS_PER_OCTAVE_DIV4_DBL[(limiter_bands as usize).min(3)];
        while (hi_lim_index as i32) <= temp_no_lim && hi_lim_index < work_limiter_band_table.len() {
            let mut div_e = 0;
            let mut oct_e = 0;
            let mut temp_e = 0;

            let k2 = work_limiter_band_table[hi_lim_index] as i32 + low_subband;
            let kx = work_limiter_band_table[lo_lim_index] as i32 + low_subband;

            let div_m = f_div_norm_fast(k2, kx, &mut div_e);

            // calculate number of octaves
            let oct_m = f_log2(div_m, div_e, &mut oct_e);

            // multiply with limiterbands per octave
            // values 1, 1.2, 2, 3 -> scale factor of 2
            let temp = f_mult_norm(oct_m, lbpo, &mut temp_e);

            // overall scale factor of temp ist addition of scalefactors from log2 calculation,
            // limiter bands scalefactor (2) and limiter bands multiplication
            temp_e += oct_e + 2;

            // div can be a maximum of 64 (k2 = 64 and kx = 1) -> oct can be a maximum of 6 ->
            // temp can be a maximum of 18 (as limiterBandsPerOctoave is a maximum factor of 3)
            // -> we need a scale factor of 5 for comparisson
            // `temp >> (5 - temp_e)`: register shift (amount modulo 32)
            if temp.wrapping_shr((5 - temp_e) as u32) < fl2fxconst_dbl(0.49f32 as f64) >> 5 {
                if work_limiter_band_table[hi_lim_index] == work_limiter_band_table[lo_lim_index] {
                    work_limiter_band_table[hi_lim_index] = high_subband as u8;
                    nb -= 1;
                    hi_lim_index += 1;
                    continue;
                }
                let mut is_patch_border = [false; 2];
                for k in 0..=no_patches.max(0) as usize {
                    if work_limiter_band_table[hi_lim_index] as i32 == patch_borders[k] {
                        is_patch_border[1] = true;
                        break;
                    }
                }
                if !is_patch_border[1] {
                    work_limiter_band_table[hi_lim_index] = high_subband as u8;
                    nb -= 1;
                    hi_lim_index += 1;
                    continue;
                }
                for k in 0..=no_patches.max(0) as usize {
                    if work_limiter_band_table[lo_lim_index] as i32 == patch_borders[k] {
                        is_patch_border[0] = true;
                        break;
                    }
                }
                if !is_patch_border[0] {
                    work_limiter_band_table[lo_lim_index] = high_subband as u8;
                    nb -= 1;
                }
            }
            lo_lim_index = hi_lim_index;
            hi_lim_index += 1;
        }
        shellsort(&mut work_limiter_band_table, sort_len);

        // Test if algorithm exceeded maximum allowed limiterbands
        if nb > MAX_NUM_LIMITERS as i32 || nb <= 0 {
            return SBRDEC_UNSUPPORTED_CONFIG;
        }

        // Restrict maximum value of limiter band table
        if work_limiter_band_table
            [(temp_no_lim.max(0) as usize).min(work_limiter_band_table.len() - 1)] as i32
            > high_subband
        {
            return SBRDEC_UNSUPPORTED_CONFIG;
        }

        // Copy limiterbands from working buffer into final destination
        let n = nb as usize + 1;
        limiter_band_table[..n].copy_from_slice(&work_limiter_band_table[..n]);
        n_bands = nb;
    }
    *no_limiter_bands = n_bands as u8;

    SBRDEC_OK
}

/// `adjustTimeSlot_EldGrid()`, optimized (see [`adjust_time_slot_eld_grid_straight`]).
///
/// The subbands are independent: subband `k` uses the noise phase index
/// `(phaseIndex + k + 1) & 511`, and until the subband with the 16th non-zero sine (included)
/// the sines of the neighbour subbands `k - 1` and `k + 1` are added with
/// `harmonicPhaseX[(lowSubband + k) & 1]` and `harmonicPhaseX[(lowSubband + k + 1) & 1]` (the
/// additions wrap, so their grouping does not matter). The sine of the last subband is added to
/// the next slot value if the last subband is in that range. On aarch64 all subbands are adjusted
/// four at a time with NEON (`fMultDiv2` of two `FIXP_DBL`: high half of `smull`; `fMult` of
/// `FIXP_SGL` and `FIXP_DBL`: `sqdmulh` with `a << 15`, which cannot saturate; the values are the
/// same on every variant), on x86 eight at a time with AVX2 (see `x86`), otherwise one by one with
/// the same formulas. The vector forms read the sines from a copy with a zero before subband 0
/// and after the last subband: the neighbour sines that the C code does not add (below subband
/// 0, above the last subband) are added as `fMultDiv2(0, x) = 0`. The last, partial vector of
/// subbands is computed on a copy of the slot values, of which only the valid lanes are written
/// back; its other lanes read gains and noise levels of the full tables. If fewer than 16 sines
/// are non-zero, the range with the neighbour sines covers all subbands (no subband follows a
/// 16th non-zero sine), so the count replaces the scan. Negative start subbands, slots shorter
/// than 64 values and ranges outside the slot or the tables use the straight port.
#[allow(clippy::too_many_arguments)]
pub fn adjust_time_slot_eld_grid(
    slot: &mut [FixpDbl],
    low_subband: i32,
    nrgs: &EnvCalcNrgs,
    ptr_harm_index: &mut u8,
    no_subbands: i32,
    scale_change: i32,
    no_noise_flag: i32,
    ptr_phase_index: &mut i32,
    scale_diff_low: i32,
) {
    let ns = no_subbands.max(0) as usize;
    if low_subband < 0 || slot.len() < 64 || ns > MAX_FREQ_COEFFS || low_subband as usize + ns > 64
    {
        return adjust_time_slot_eld_grid_straight(
            slot,
            low_subband,
            nrgs,
            ptr_harm_index,
            no_subbands,
            scale_change,
            no_noise_flag,
            ptr_phase_index,
            scale_diff_low,
        );
    }
    let p_gain = &nrgs.nrg_gain[..ns];
    let p_noise_level = &nrgs.noise_level[..ns];
    let p_sine_level = &nrgs.nrg_sine;
    let phase_index = *ptr_phase_index;
    let harm_index = *ptr_harm_index as usize & 3;
    let hpx = &HARMONIC_PHASE_X[harm_index];
    let hp0 = HARMONIC_PHASE[harm_index][0];
    let max_val = MAX_VAL_NRG_HEADROOM.wrapping_shr(scale_change as u32); // (register shift)
    let min_val = -max_val;
    let lo = low_subband as usize;

    if lo >= 1 {
        let v = saturate_shift(
            f_mult_div2(hpx[(low_subband & 1) as usize], p_sine_level[0]),
            scale_diff_low,
            DFRACT_BITS,
        );
        slot[lo - 1] = f_add_saturate(slot[lo - 1], v);
    }

    // the subbands up to k_tone (excluded) get the neighbour sines
    let sine = &p_sine_level[..ns];
    let k_tone = if sine.iter().filter(|&&s| s != 0).count() < 16 {
        ns
    } else {
        let mut k_tone = 0;
        let mut tones = 0;
        for k in 0..ns {
            if tones < 16 {
                k_tone = k + 1;
            }
            tones += (p_sine_level[k] != 0) as i32;
        }
        k_tone
    };
    let e = AdjEld {
        lo,
        ns,
        phase_index,
        hp0,
        hpx: *hpx,
        k_tone,
        noise_on: no_noise_flag == 0,
        max_val,
        min_val,
        sc: (scale_change as u32 & 31) as i32,
    };
    let slots = &mut slot[lo..lo + ns];
    #[cfg(target_arch = "aarch64")]
    let simd = {
        neon_eld::all(slots, &nrgs.nrg_gain, &nrgs.noise_level, sine, p_sine_level, &e);
        true
    };
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    let simd = fdk_aac_rs_fdk::simd_x86::avx2() && {
        // SAFETY: the CPU has AVX2.
        unsafe { x86::eld_all(slots, &nrgs.nrg_gain, &nrgs.noise_level, sine, &e) };
        true
    };
    #[cfg(not(any(target_arch = "aarch64", target_arch = "x86", target_arch = "x86_64")))]
    let simd = false;
    if !simd {
        for k in 0..ns {
            slots[k] = e.subband(k, slots[k], p_gain[k], p_noise_level[k], sine);
        }
    }
    // the sine of the last subband in the next slot value
    if ns >= 1 && ns <= k_tone && ns as i32 + low_subband < 63 {
        let v = f_mult_div2(sine[ns - 1], hpx[(lo + ns) & 1]);
        slot[lo + ns] = slot[lo + ns].wrapping_add(v);
    }

    *ptr_harm_index = ((harm_index + 1) & 3) as u8;
    *ptr_phase_index = (phase_index + ns as i32) & (SBR_NF_NO_RANDOM_VAL - 1);
}

/// The parameters of [`adjust_time_slot_eld_grid`] that are constant over the subbands.
struct AdjEld {
    lo: usize,
    ns: usize,
    phase_index: i32,
    hp0: i32,
    hpx: [FixpDbl; 2],
    k_tone: usize,
    noise_on: bool,
    max_val: FixpDbl,
    min_val: FixpDbl,
    sc: i32,
}

impl AdjEld {
    /// Subband `k` (the slot value `x`).
    #[inline(always)]
    fn subband(
        &self,
        k: usize,
        x: FixpDbl,
        gain: FixpDbl,
        noise: FixpDbl,
        sine: &[FixpDbl],
    ) -> FixpDbl {
        let s = sine[k];
        let mut sig = f_max(f_min(f_mult_div2(x, gain), self.max_val), self.min_val)
            .wrapping_shl(self.sc as u32);
        if s == 0 && self.noise_on {
            let index = ((self.phase_index + k as i32 + 1) & (SBR_NF_NO_RANDOM_VAL - 1)) as usize;
            sig = sig.wrapping_add(f_mult(FDK_SBR_DECODER_SBR_RANDOM_PHASE[index][0], noise));
        }
        sig = sig.wrapping_add(s.wrapping_mul(self.hp0));
        if k < self.k_tone {
            if k >= 1 {
                sig = f_mult_add_div2(sig, sine[k - 1], self.hpx[(self.lo + k) & 1]);
            }
            if k + 1 < self.ns {
                sig = f_mult_add_div2(sig, sine[k + 1], self.hpx[(self.lo + k + 1) & 1]);
            }
        }
        sig
    }
}

/// NEON kernel of [`adjust_time_slot_eld_grid`].
#[cfg(target_arch = "aarch64")]
mod neon_eld {
    use super::*;
    use core::arch::aarch64::*;

    /// All `ns` subbands four at a time (see [`adjust_time_slot_eld_grid`]): `slots`, `sine`
    /// have `ns <= MAX_FREQ_COEFFS` values, `gain`, `noise` and `sine_full` (whose first `ns`
    /// values are `sine`) are the full tables.
    pub(super) fn all(
        slots: &mut [FixpDbl],
        gain: &[FixpDbl; MAX_FREQ_COEFFS],
        noise: &[FixpDbl; MAX_FREQ_COEFFS],
        sine: &[FixpDbl],
        sine_full: &[FixpDbl; MAX_FREQ_COEFFS],
        e: &AdjEld,
    ) {
        let ns = slots.len();
        assert!(sine.len() == ns && ns <= MAX_FREQ_COEFFS && MAX_FREQ_COEFFS % 4 == 0);
        // sp[k + 1] = sine[k], zero before and after (filled with fixed-size vector code, not
        // with calls of memset/memcpy for the variable length)
        let mut sp = [0 as FixpDbl; MAX_FREQ_COEFFS + 8];
        debug_assert!(sine_full[..ns] == *sine);
        // SAFETY: NEON is part of aarch64; the blocks read sine_full[q..q + 4] and write
        // sp[1 + q..5 + q] for q + 4 <= MAX_FREQ_COEFFS.
        unsafe {
            let lanes: [i32; 4] = [0, 1, 2, 3];
            let lanes = vld1q_s32(lanes.as_ptr());
            for q in (0..MAX_FREQ_COEFFS).step_by(4) {
                let keep =
                    vcltq_s32(vaddq_s32(lanes, vdupq_n_s32(q as i32)), vdupq_n_s32(ns as i32));
                let v =
                    vandq_s32(vld1q_s32(sine_full.as_ptr().add(q)), vreinterpretq_s32_u32(keep));
                vst1q_s32(sp.as_mut_ptr().add(1 + q), v);
            }
        }
        let full = ns & !3;
        // SAFETY: NEON is part of aarch64. A block at k (a multiple of 4, k < ns) loads
        // gain[k..k + 4], noise[k..k + 4] (k + 4 <= MAX_FREQ_COEFFS) and sp[k..k + 6]; the full
        // blocks load and store slots[k..k + 4] (k + 4 <= full <= ns), the last one a local copy.
        unsafe {
            let ld = |x: *const FixpDbl, k: usize| vld1q_s32(x.add(k));
            let dd = |a: int32x4_t, b: int32x4_t| {
                let lo = vreinterpretq_s32_s64(vmull_s32(vget_low_s32(a), vget_low_s32(b)));
                let hi = vreinterpretq_s32_s64(vmull_high_s32(a, b));
                vuzp2q_s32(lo, hi)
            };
            let (maxv, minv) = (vdupq_n_s32(e.max_val), vdupq_n_s32(e.min_val));
            let sc = vdupq_n_s32(e.sc);
            let hp0 = vdupq_n_s32(e.hp0);
            let zero = vdupq_n_s32(0);
            let noise_on = vdupq_n_u32(if e.noise_on { u32::MAX } else { 0 });
            let lanes: [u32; 4] = [0, 1, 2, 3];
            let lanes = vld1q_u32(lanes.as_ptr());
            let k_tone = vdupq_n_u32(e.k_tone as u32);
            // harmonicPhaseX of the lower and the upper neighbour per lane (k is a multiple of 4,
            // so the parity of lane l is that of lo + l)
            let px = |o: usize| -> [i32; 4] { core::array::from_fn(|l| e.hpx[(e.lo + l + o) & 1]) };
            let (pxl, pxu) = (px(0), px(1));
            let (pxl, pxu) = (vld1q_s32(pxl.as_ptr()), vld1q_s32(pxu.as_ptr()));
            let (gp, np, sp) = (gain.as_ptr(), noise.as_ptr(), sp.as_ptr());
            let block = |k: usize, x: int32x4_t| -> int32x4_t {
                let s = ld(sp, k + 1);
                let mut sig = vshlq_s32(vmaxq_s32(vminq_s32(dd(x, ld(gp, k)), maxv), minv), sc);
                let mut ph = [0i32; 4];
                for (l, v) in ph.iter_mut().enumerate() {
                    let index = (e.phase_index + (k + l) as i32 + 1) & (SBR_NF_NO_RANDOM_VAL - 1);
                    *v = (FDK_SBR_DECODER_SBR_RANDOM_PHASE[index as usize][0] as i32) << 15;
                }
                let nterm = vshlq_n_s32::<1>(vqdmulhq_s32(ld(np, k), vld1q_s32(ph.as_ptr())));
                let use_noise = vandq_u32(vceqzq_s32(s), noise_on);
                sig = vaddq_s32(sig, vbslq_s32(use_noise, nterm, zero));
                sig = vaddq_s32(sig, vmulq_s32(s, hp0));
                let nb = vaddq_s32(dd(ld(sp, k), pxl), dd(ld(sp, k + 2), pxu));
                let on = vcltq_u32(vaddq_u32(lanes, vdupq_n_u32(k as u32)), k_tone);
                vaddq_s32(sig, vbslq_s32(on, nb, zero))
            };
            let mut k = 0;
            while k < full {
                let p = slots.as_mut_ptr().add(k);
                vst1q_s32(p, block(k, vld1q_s32(p)));
                k += 4;
            }
            if full < ns {
                // the last 1..=3 subbands through a local block (element copies, not memcpy)
                let mut t = [0 as FixpDbl; 4];
                for i in 0..3 {
                    if full + i < ns {
                        t[i] = slots[full + i];
                    }
                }
                vst1q_s32(t.as_mut_ptr(), block(full, vld1q_s32(t.as_ptr())));
                for i in 0..3 {
                    if full + i < ns {
                        slots[full + i] = t[i];
                    }
                }
            }
        }
    }
}

/// `adjustTimeSlotLC()`, optimized (see [`adjust_time_slot_lc_straight`]).
///
/// The first and the last subband are processed as in the C code. The subbands in between are
/// independent: subband `j` uses the noise phase index `(index + j) & 511` (`index` after the
/// first subband; the C code increments it per subband and masks it only where it is used), its
/// harmonic sign depends on the parity of `j`, and with odd harmonic indices the neighbour sines
/// are added while the number of non-zero sines up to `j` is at most 16. On aarch64 four of them
/// are adjusted at a time with NEON, the multiplications (`fMultDiv2` of two `FIXP_DBL`: high half
/// of `smull`; with a `FIXP_SGL`: `sqdmulh` with `a << 15`, which cannot saturate) give the same
/// values on every variant. Fewer than 3 subbands, ranges outside the slot or the tables, and
/// negative start subbands use the straight port.
#[allow(clippy::too_many_arguments)]
pub fn adjust_time_slot_lc(
    slot: &mut [FixpDbl],
    low_subband: i32,
    nrgs: &EnvCalcNrgs,
    ptr_harm_index: &mut u8,
    no_subbands: i32,
    scale_change: i32,
    no_noise_flag: i32,
    ptr_phase_index: &mut i32,
) {
    if no_subbands <= 2
        || low_subband < 0
        || no_subbands as usize > MAX_FREQ_COEFFS
        || (low_subband + no_subbands) as usize > slot.len()
    {
        return adjust_time_slot_lc_straight(
            slot,
            low_subband,
            nrgs,
            ptr_harm_index,
            no_subbands,
            scale_change,
            no_noise_flag,
            ptr_phase_index,
        );
    }
    let ns = no_subbands as usize;
    let p_gain = &nrgs.nrg_gain[..ns];
    let p_noise_level = &nrgs.noise_level[..ns];
    let p_sine = &nrgs.nrg_sine[..ns];

    let mut index = *ptr_phase_index;
    let harm_index = *ptr_harm_index;
    let mut freq_inv_flag = (low_subband & 1) != 0;
    let mut tone_count = 0;
    let mut sine_sign = true;
    let max_val = MAX_VAL_NRG_HEADROOM.wrapping_shr(scale_change as u32); // (register shift)
    let min_val = -max_val;
    let sc = scale_change as u32 & 31;
    let adj =
        |x: FixpDbl, g: FixpDbl| f_max(f_min(f_mult_div2(x, g), max_val), min_val).wrapping_shl(sc);
    let noise_on = no_noise_flag == 0;
    let lo = low_subband as usize;
    let mut p = lo; // ptrReal (the first subband; the middle ones are slot[lo + j])

    // First subband (as in the C code)
    index = (index + 1) & (SBR_NF_NO_RANDOM_VAL - 1);
    let mut signal_real = adj(slot[p], p_gain[0]);
    let mut sine_level = p_sine[0];
    let sine_level_next = p_sine[1];
    if sine_level != 0 {
        tone_count += 1;
    } else if noise_on {
        signal_real = signal_real.wrapping_add(f_mult(
            FDK_SBR_DECODER_SBR_RANDOM_PHASE[index as usize][0],
            p_noise_level[0],
        ));
    }
    if harm_index & 0x1 == 0 {
        signal_real = signal_real.wrapping_add(if harm_index & 0x2 != 0 {
            sine_level.wrapping_neg()
        } else {
            sine_level
        });
        slot[p] = signal_real;
    } else {
        let mut shift = scale_change + 1;
        shift = if shift >= 0 {
            fix_min(DFRACT_BITS - 1, shift)
        } else {
            fix_max(-(DFRACT_BITS - 1), shift)
        };
        let tmp1 = if shift >= 0 {
            f_mult_div2(C1, sine_level) >> shift
        } else {
            f_mult_div2(C1, sine_level) << (-shift)
        };
        let tmp2 = f_mult_div2(C1, sine_level_next);
        if (((harm_index >> 1) & 0x1) != 0) ^ freq_inv_flag {
            if p >= 1 {
                slot[p - 1] = f_add_saturate(slot[p - 1], tmp1);
            }
            signal_real = signal_real.wrapping_sub(tmp2);
        } else {
            if p >= 1 {
                slot[p - 1] = f_add_saturate(slot[p - 1], tmp1.wrapping_neg());
            }
            signal_real = signal_real.wrapping_add(tmp2);
        }
        slot[p] = signal_real;
        freq_inv_flag = !freq_inv_flag;
    }

    // Subbands 1..=ns-2
    let odd = harm_index & 0x1 != 0;
    if !odd {
        if harm_index == 0 {
            sine_sign = false;
        }
    } else if harm_index == 1 {
        freq_inv_flag = !freq_inv_flag;
    }
    // with odd harmonic indices, the neighbour sines are added to the subbands below `j_tone`
    // (the subband with the 17th non-zero sine)
    let mut j_tone = 1;
    let mut tones = tone_count;
    if odd {
        for j in 1..ns - 1 {
            tones += (p_sine[j] != 0) as i32;
            if tones <= 16 {
                j_tone = j + 1;
            }
        }
    }
    let mid = AdjLcMid {
        index,
        odd,
        sine_sign,
        freq_inv1: freq_inv_flag,
        j_tone,
        noise_on,
        max_val,
        min_val,
        sc: sc as i32,
    };
    let (slots, gains, noises) = (&mut slot[lo..lo + ns], p_gain, p_noise_level);
    #[cfg(target_arch = "aarch64")]
    let j0 = neon_lc::mid(slots, gains, noises, p_sine, &mid);
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    let j0 = if fdk_aac_rs_fdk::simd_x86::avx2() {
        // SAFETY: the CPU has AVX2.
        unsafe { x86::lc_mid(slots, gains, noises, p_sine, &mid) }
    } else {
        1
    };
    #[cfg(not(any(target_arch = "aarch64", target_arch = "x86", target_arch = "x86_64")))]
    let j0 = 1;
    for j in j0..ns - 1 {
        slots[j] = mid.subband(j, slots[j], gains[j], noises[j], p_sine);
    }
    if odd {
        tone_count = tones;
        if (ns - 2) & 1 != 0 {
            freq_inv_flag = !freq_inv_flag;
        }
    }
    index += (ns - 2) as i32;
    p += ns - 1;

    // Last subband (as in the C code)
    {
        index += 1;
        signal_real = adj(slot[p], p_gain[ns - 1]);
        let sine_level_prev = f_mult_div2(p_sine[ns - 2], fl2fx_sgl(0.0163f32));
        sine_level = p_sine[ns - 1];

        if sine_level != 0 {
            tone_count += 1;
        } else if noise_on {
            index &= SBR_NF_NO_RANDOM_VAL - 1;
            signal_real = signal_real.wrapping_add(f_mult(
                FDK_SBR_DECODER_SBR_RANDOM_PHASE[index as usize][0],
                p_noise_level[ns - 1],
            ));
        }

        if !odd {
            slot[p] = signal_real.wrapping_add(if sine_sign {
                sine_level.wrapping_neg()
            } else {
                sine_level
            });
        } else if tone_count <= 16 {
            if freq_inv_flag {
                slot[p] = signal_real.wrapping_sub(sine_level_prev);
                p += 1;
                if no_subbands + low_subband < 63 {
                    if let Some(v) = slot.get_mut(p) {
                        *v = v.wrapping_add(f_mult_div2(C1, sine_level));
                    }
                }
            } else {
                slot[p] = signal_real.wrapping_add(sine_level_prev);
                p += 1;
                if no_subbands + low_subband < 63 {
                    if let Some(v) = slot.get_mut(p) {
                        *v = v.wrapping_sub(f_mult_div2(C1, sine_level));
                    }
                }
            }
        } else {
            slot[p] = signal_real;
        }
    }
    *ptr_harm_index = (harm_index.wrapping_add(1)) & 3;
    *ptr_phase_index = index & (SBR_NF_NO_RANDOM_VAL - 1);
}

/// The state of the middle subbands of [`adjust_time_slot_lc`].
struct AdjLcMid {
    /// the phase index after the first subband
    index: i32,
    /// odd harmonic index
    odd: bool,
    sine_sign: bool,
    /// freqInvFlag of subband 1
    freq_inv1: bool,
    /// the first subband without neighbour sines (odd harmonic indices)
    j_tone: usize,
    noise_on: bool,
    max_val: FixpDbl,
    min_val: FixpDbl,
    sc: i32,
}

impl AdjLcMid {
    /// Subband `j` (1..=ns-2) of the slot (the value `x`).
    #[inline(always)]
    fn subband(
        &self,
        j: usize,
        x: FixpDbl,
        gain: FixpDbl,
        noise: FixpDbl,
        sine: &[FixpDbl],
    ) -> FixpDbl {
        let s = sine[j];
        let adjusted = f_max(f_min(f_mult_div2(x, gain), self.max_val), self.min_val)
            .wrapping_shl(self.sc as u32);
        let noise_term = |sig: FixpDbl| {
            let index = ((self.index + j as i32) & (SBR_NF_NO_RANDOM_VAL - 1)) as usize;
            sig.wrapping_add(f_mult(FDK_SBR_DECODER_SBR_RANDOM_PHASE[index][0], noise))
        };
        if !self.odd {
            let mut sig = if self.sine_sign { s.wrapping_neg() } else { s };
            if sig == 0 && self.noise_on {
                sig = noise_term(sig);
            }
            sig.wrapping_add(adjusted)
        } else {
            let mut sig = adjusted;
            if s == 0 && self.noise_on {
                sig = noise_term(sig);
            }
            if j < self.j_tone {
                let add_sine = f_mult_div2(sine[j - 1].wrapping_sub(sine[j + 1]), C1);
                let inv = self.freq_inv1 ^ ((j - 1) & 1 != 0);
                sig = sig.wrapping_add(if inv { add_sine.wrapping_neg() } else { add_sine });
            }
            sig
        }
    }
}

/// NEON kernel of [`adjust_time_slot_lc`].
#[cfg(target_arch = "aarch64")]
mod neon_lc {
    use super::*;
    use core::arch::aarch64::*;

    /// Middle subbands `1..j_end` four at a time (`j_end - 1` a multiple of 4, `j_end <= ns - 1`
    /// so that `sine[j + 1]` exists); returns `j_end`. All slices have `ns` values.
    pub(super) fn mid(
        slots: &mut [FixpDbl],
        gain: &[FixpDbl],
        noise: &[FixpDbl],
        sine: &[FixpDbl],
        m: &AdjLcMid,
    ) -> usize {
        let ns = slots.len();
        assert!(gain.len() == ns && noise.len() == ns && sine.len() == ns && ns >= 3);
        let j_end = 1 + ((ns - 2) & !3);
        // SAFETY: NEON is part of aarch64; the loads and stores cover [j - 1, j + 5) with
        // j + 4 <= j_end <= ns - 1.
        unsafe {
            let ld = |x: &[FixpDbl], k: usize| vld1q_s32(x.as_ptr().add(k));
            let dd = |a: int32x4_t, b: int32x4_t| {
                let lo = vreinterpretq_s32_s64(vmull_s32(vget_low_s32(a), vget_low_s32(b)));
                let hi = vreinterpretq_s32_s64(vmull_high_s32(a, b));
                vuzp2q_s32(lo, hi)
            };
            let (maxv, minv) = (vdupq_n_s32(m.max_val), vdupq_n_s32(m.min_val));
            let sc = vdupq_n_s32(m.sc);
            let c1 = vdupq_n_s32((C1 as i32) << 15);
            let zero = vdupq_n_s32(0);
            let noise_on = vdupq_n_u32(if m.noise_on { u32::MAX } else { 0 });
            let lanes: [u32; 4] = [0, 1, 2, 3];
            let lanes = vld1q_u32(lanes.as_ptr());
            let j_tone = vdupq_n_u32(m.j_tone as u32);
            let mut j = 1;
            while j < j_end {
                let s = ld(sine, j);
                let adjusted =
                    vshlq_s32(vmaxq_s32(vminq_s32(dd(ld(slots, j), ld(gain, j)), maxv), minv), sc);
                let mut ph = [0i32; 4];
                for (l, v) in ph.iter_mut().enumerate() {
                    let index = (m.index + (j + l) as i32) & (SBR_NF_NO_RANDOM_VAL - 1);
                    *v = (FDK_SBR_DECODER_SBR_RANDOM_PHASE[index as usize][0] as i32) << 15;
                }
                let nterm = vshlq_n_s32::<1>(vqdmulhq_s32(ld(noise, j), vld1q_s32(ph.as_ptr())));
                let no_sine = vceqzq_s32(s);
                let use_noise = vandq_u32(no_sine, noise_on);
                let out = if !m.odd {
                    let sig = if m.sine_sign { vnegq_s32(s) } else { s };
                    vaddq_s32(vbslq_s32(use_noise, nterm, sig), adjusted)
                } else {
                    let sig = vaddq_s32(adjusted, vbslq_s32(use_noise, nterm, zero));
                    let add = vqdmulhq_s32(vsubq_s32(ld(sine, j - 1), ld(sine, j + 1)), c1);
                    // inv for subband j + l: freq_inv1 ^ ((j + l - 1) odd)
                    let mut sign = [0i32; 4];
                    for (l, v) in sign.iter_mut().enumerate() {
                        *v = if m.freq_inv1 ^ ((j + l - 1) & 1 != 0) { -1 } else { 1 };
                    }
                    let add = vmulq_s32(add, vld1q_s32(sign.as_ptr()));
                    let on = vcltq_u32(vaddq_u32(lanes, vdupq_n_u32(j as u32)), j_tone);
                    vaddq_s32(sig, vbslq_s32(on, add, zero))
                };
                vst1q_s32(slots.as_mut_ptr().add(j), out);
                j += 4;
            }
        }
        j_end
    }
}

/// [`adjust_time_slot_hq`] and [`adjust_time_slot_hq_gain_and_noise`], optimized.
///
/// The subbands are independent: the noise phase index of subband `k` is
/// `(phaseIndex + k + 1) & (SBR_NF_NO_RANDOM_VAL - 1)` (the C code increments the index for
/// every subband and masks it only where it is used; masking subtracts multiples of 512), and
/// the harmonic sign of subband `k` depends on the parity of `lowSubband + k`. On aarch64 four
/// subbands are adjusted at a time with NEON; all multiplications of these functions
/// (`fMultDiv2` of two `FIXP_DBL`, `fMult`/`fMultDiv2` of a `FIXP_SGL` and a `FIXP_DBL`) give
/// the same values on every variant: the high half of `smull`, and `sqdmulh(x, a << 15)` for
/// `fMultDiv2(FIXP_SGL a, x)` (`a << 15` is never `INT_MIN`, so it cannot saturate). The
/// remaining subbands (on other targets all of them) are computed one by one with the same
/// formulas.
#[allow(clippy::too_many_arguments)]
fn adjust_time_slot_hq_fast(
    ptr_real: &mut [FixpDbl],
    ptr_imag: &mut [FixpDbl],
    h_sbr_cal_env: &mut SbrCalculateEnvelope,
    nrgs: &EnvCalcNrgs,
    low_subband: i32,
    no_subbands: i32,
    scale_change: i32,
    smooth_ratio: FixpSgl,
    no_noise_flag: i32,
    mut filt_buffer_noise_shift: i32,
    harmonics: bool,
) {
    let direct_ratio: FixpSgl = (MAXVAL_SGL as FixpSgl).wrapping_sub(smooth_ratio);
    let index0 = h_sbr_cal_env.phase_index;
    let harm_index = h_sbr_cal_env.harm_index;
    let freq_inv0 = low_subband & 1;
    let max_val = MAX_VAL_NRG_HEADROOM.wrapping_shr(scale_change as u32); // (register shift)
    let min_val = -max_val;

    h_sbr_cal_env.phase_index = (index0 + no_subbands) & (SBR_NF_NO_RANDOM_VAL - 1);
    if harmonics {
        h_sbr_cal_env.harm_index = harm_index.wrapping_add(1) & 3;
    }

    filt_buffer_noise_shift += 1; // due to later use of fMultDiv2 instead of fMult
    let (shift, max_val_noise) = if filt_buffer_noise_shift < 0 {
        (fix_min(DFRACT_BITS - 1, -filt_buffer_noise_shift), 0)
    } else {
        let shift = fix_min(DFRACT_BITS - 1, filt_buffer_noise_shift);
        (shift, MAX_VAL_NRG_HEADROOM >> shift)
    };
    let min_val_noise = -max_val_noise;

    let n = (no_subbands.max(0) as usize).min(ptr_real.len()).min(ptr_imag.len());
    let gain = &nrgs.nrg_gain[..n];
    let noise_level = &nrgs.noise_level[..n];
    let sine = &nrgs.nrg_sine[..n];
    let filt_buffer = &h_sbr_cal_env.filt_buffer[..n];
    let filt_buffer_noise = &h_sbr_cal_env.filt_buffer_noise[..n];
    let (re, im) = (&mut ptr_real[..n], &mut ptr_imag[..n]);

    let p = AdjHq {
        smooth_ratio,
        direct_ratio,
        noise_shift_neg: filt_buffer_noise_shift < 0,
        shift,
        max_val_noise,
        min_val_noise,
        max_val,
        min_val,
        scale_change_amt: (scale_change as u32 & 31) as i32,
        index0,
        harm_index: if harmonics { Some(harm_index) } else { None },
        freq_inv0,
        noise_on: no_noise_flag == 0,
    };

    #[cfg(target_arch = "aarch64")]
    let k0 = {
        let k0 = n & !3;
        neon_adj::hq(re, im, gain, noise_level, sine, filt_buffer, filt_buffer_noise, k0, &p);
        k0
    };
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    let k0 = if fdk_aac_rs_fdk::simd_x86::avx2() {
        let k0 = n & !7;
        // SAFETY: the CPU has AVX2.
        unsafe { x86::hq(re, im, gain, noise_level, sine, filt_buffer, filt_buffer_noise, k0, &p) };
        k0
    } else {
        0
    };
    #[cfg(not(any(target_arch = "aarch64", target_arch = "x86", target_arch = "x86_64")))]
    let k0 = 0;
    for k in k0..n {
        let (sg, sn) = p.smoothed(gain[k], noise_level[k], filt_buffer[k], filt_buffer_noise[k]);
        let mut signal_real = p.adjusted(re[k], sg);
        let mut signal_imag = p.adjusted(im[k], sg);
        if sine[k] != 0 {
            let (ar, ai) = p.harmonic(k, sine[k]);
            signal_real = signal_real.wrapping_add(ar);
            signal_imag = signal_imag.wrapping_add(ai);
        } else if p.noise_on {
            let index = ((p.index0 + k as i32 + 1) & (SBR_NF_NO_RANDOM_VAL - 1)) as usize;
            let ph = FDK_SBR_DECODER_SBR_RANDOM_PHASE[index];
            signal_real = signal_real.wrapping_add(f_mult(ph[0], sn));
            signal_imag = signal_imag.wrapping_add(f_mult(ph[1], sn));
        }
        re[k] = signal_real;
        im[k] = signal_imag;
    }
}

/// The parameters of [`adjust_time_slot_hq_fast`] that are constant over the subbands.
struct AdjHq {
    smooth_ratio: FixpSgl,
    direct_ratio: FixpSgl,
    noise_shift_neg: bool,
    shift: i32,
    max_val_noise: FixpDbl,
    min_val_noise: FixpDbl,
    max_val: FixpDbl,
    min_val: FixpDbl,
    scale_change_amt: i32,
    index0: i32,
    harm_index: Option<u8>,
    freq_inv0: i32,
    noise_on: bool,
}

impl AdjHq {
    /// The smoothed gain and noise level of a subband.
    #[inline(always)]
    fn smoothed(
        &self,
        gain: FixpDbl,
        noise: FixpDbl,
        fb: FixpDbl,
        fbn: FixpDbl,
    ) -> (FixpDbl, FixpDbl) {
        if self.smooth_ratio <= 0 {
            return (gain, noise);
        }
        let sg = f_mult(self.smooth_ratio, fb).wrapping_add(f_mult(self.direct_ratio, gain));
        let sn = if self.noise_shift_neg {
            (f_mult_div2(self.smooth_ratio, fbn) >> self.shift)
                .wrapping_add(f_mult(self.direct_ratio, noise))
        } else {
            let t = f_mult_div2(self.smooth_ratio, fbn);
            (f_max(f_min(t, self.max_val_noise), self.min_val_noise) << self.shift)
                .wrapping_add(f_mult(self.direct_ratio, noise))
        };
        (sg, f_max(f_min(sn, MAXVAL_DBL / 2), MINVAL_DBL / 2))
    }

    /// The envelope adjustment of one value.
    #[inline(always)]
    fn adjusted(&self, x: FixpDbl, gain: FixpDbl) -> FixpDbl {
        f_max(f_min(f_mult_div2(x, gain), self.max_val), self.min_val)
            .wrapping_shl(self.scale_change_amt as u32)
    }

    /// The sine added to the real and imaginary part of subband `k` (with harmonics).
    #[inline(always)]
    fn harmonic(&self, k: usize, sine: FixpDbl) -> (FixpDbl, FixpDbl) {
        let inv = ((self.freq_inv0 + k as i32) & 1) != 0;
        match self.harm_index() {
            Some(0) => (sine, 0),
            Some(2) => (sine.wrapping_neg(), 0),
            Some(1) => (0, if inv { sine.wrapping_neg() } else { sine }),
            Some(_) => (0, if inv { sine } else { sine.wrapping_neg() }),
            None => (0, 0),
        }
    }

    /// The harmonic index as the loops of the C code see it: values above 3 (not reachable,
    /// the index is kept below 4) count as 3 with smoothing and add no sine without.
    #[inline(always)]
    fn harm_index(&self) -> Option<u8> {
        match self.harm_index {
            Some(h) if h > 3 && self.smooth_ratio <= 0 => None,
            h => h,
        }
    }
}

/// `adjustTimeSlotHQ_GainAndNoise()`, optimized (see `adjust_time_slot_hq_fast`).
#[allow(clippy::too_many_arguments)]
pub fn adjust_time_slot_hq_gain_and_noise(
    ptr_real: &mut [FixpDbl],
    ptr_imag: &mut [FixpDbl],
    h_sbr_cal_env: &mut SbrCalculateEnvelope,
    nrgs: &EnvCalcNrgs,
    low_subband: i32,
    no_subbands: i32,
    scale_change: i32,
    smooth_ratio: FixpSgl,
    no_noise_flag: i32,
    filt_buffer_noise_shift: i32,
) {
    adjust_time_slot_hq_fast(
        ptr_real,
        ptr_imag,
        h_sbr_cal_env,
        nrgs,
        low_subband,
        no_subbands,
        scale_change,
        smooth_ratio,
        no_noise_flag,
        filt_buffer_noise_shift,
        false,
    );
}

/// `adjustTimeSlotHQ()`, optimized (see `adjust_time_slot_hq_fast`).
#[allow(clippy::too_many_arguments)]
pub fn adjust_time_slot_hq(
    ptr_real: &mut [FixpDbl],
    ptr_imag: &mut [FixpDbl],
    h_sbr_cal_env: &mut SbrCalculateEnvelope,
    nrgs: &EnvCalcNrgs,
    low_subband: i32,
    no_subbands: i32,
    scale_change: i32,
    smooth_ratio: FixpSgl,
    no_noise_flag: i32,
    filt_buffer_noise_shift: i32,
) {
    adjust_time_slot_hq_fast(
        ptr_real,
        ptr_imag,
        h_sbr_cal_env,
        nrgs,
        low_subband,
        no_subbands,
        scale_change,
        smooth_ratio,
        no_noise_flag,
        filt_buffer_noise_shift,
        true,
    );
}

/// NEON kernel of [`adjust_time_slot_hq_fast`].
#[cfg(target_arch = "aarch64")]
mod neon_adj {
    use super::*;
    use core::arch::aarch64::*;

    /// Subbands `0..k_end` (a multiple of 4; all slices have at least `k_end` values).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn hq(
        re: &mut [FixpDbl],
        im: &mut [FixpDbl],
        gain: &[FixpDbl],
        noise: &[FixpDbl],
        sine: &[FixpDbl],
        fb: &[FixpDbl],
        fbn: &[FixpDbl],
        k_end: usize,
        p: &AdjHq,
    ) {
        assert!([re.len(), im.len(), gain.len(), noise.len(), sine.len(), fb.len(), fbn.len()]
            .iter()
            .all(|&l| k_end <= l));
        // SAFETY: NEON is part of aarch64; every load and store covers [k, k + 4) with
        // k + 4 <= k_end.
        unsafe {
            let ld = |x: &[FixpDbl], k: usize| vld1q_s32(x.as_ptr().add(k));
            // fMultDiv2(FIXP_SGL a, x) with a2 = a << 15 in every lane
            let sd = |x: int32x4_t, a2: int32x4_t| vqdmulhq_s32(x, a2);
            // fMultDiv2(FIXP_DBL, FIXP_DBL): the high half of the product
            let dd = |a: int32x4_t, b: int32x4_t| {
                let lo = vreinterpretq_s32_s64(vmull_s32(vget_low_s32(a), vget_low_s32(b)));
                let hi = vreinterpretq_s32_s64(vmull_high_s32(a, b));
                vuzp2q_s32(lo, hi)
            };
            let sr2 = vdupq_n_s32((p.smooth_ratio as i32) << 15);
            let dr2 = vdupq_n_s32((p.direct_ratio as i32) << 15);
            let (maxv, minv) = (vdupq_n_s32(p.max_val), vdupq_n_s32(p.min_val));
            let (maxn, minn) = (vdupq_n_s32(p.max_val_noise), vdupq_n_s32(p.min_val_noise));
            let (maxh, minh) = (vdupq_n_s32(MAXVAL_DBL / 2), vdupq_n_s32(MINVAL_DBL / 2));
            let nsh = vdupq_n_s32(if p.noise_shift_neg { -p.shift } else { p.shift });
            let sc = vdupq_n_s32(p.scale_change_amt);
            let zero = vdupq_n_s32(0);
            // harmonics: the sine is added to the real part with the sign sr, to the imaginary
            // part with the sign si (per lane; k is a multiple of 4, so the parity of the lane
            // is the parity of k)
            let lane_inv: [i32; 4] = core::array::from_fn(|j| (p.freq_inv0 + j as i32) & 1);
            let (sr, si): ([i32; 4], [i32; 4]) = match p.harm_index() {
                Some(0) => ([1; 4], [0; 4]),
                Some(2) => ([-1; 4], [0; 4]),
                Some(1) => ([0; 4], lane_inv.map(|v| if v != 0 { -1 } else { 1 })),
                Some(_) => ([0; 4], lane_inv.map(|v| if v != 0 { 1 } else { -1 })),
                None => ([0; 4], [0; 4]),
            };
            let (sr, si) = (vld1q_s32(sr.as_ptr()), vld1q_s32(si.as_ptr()));
            let noise_on = vdupq_n_u32(if p.noise_on { u32::MAX } else { 0 });

            let mut k = 0;
            while k < k_end {
                let (g, nl) = (ld(gain, k), ld(noise, k));
                let (sg, sn) = if p.smooth_ratio > 0 {
                    let sg = vaddq_s32(
                        vshlq_n_s32::<1>(sd(ld(fb, k), sr2)),
                        vshlq_n_s32::<1>(sd(g, dr2)),
                    );
                    let t = sd(ld(fbn, k), sr2);
                    let t = if p.noise_shift_neg { t } else { vmaxq_s32(vminq_s32(t, maxn), minn) };
                    let sn = vaddq_s32(vshlq_s32(t, nsh), vshlq_n_s32::<1>(sd(nl, dr2)));
                    (sg, vmaxq_s32(vminq_s32(sn, maxh), minh))
                } else {
                    (g, nl)
                };
                let adj = |x: int32x4_t| vshlq_s32(vmaxq_s32(vminq_s32(dd(x, sg), maxv), minv), sc);
                let sig_r = adj(ld(re, k));
                let sig_i = adj(ld(im, k));

                let s = ld(sine, k);
                let has_sine = vtstq_s32(s, s);
                // noise: the random phases of the four subbands
                let mut ph_r = [0i32; 4];
                let mut ph_i = [0i32; 4];
                for j in 0..4 {
                    let index = (p.index0 + (k + j) as i32 + 1) & (SBR_NF_NO_RANDOM_VAL - 1);
                    let ph = FDK_SBR_DECODER_SBR_RANDOM_PHASE[index as usize];
                    ph_r[j] = (ph[0] as i32) << 15;
                    ph_i[j] = (ph[1] as i32) << 15;
                }
                let nr = vshlq_n_s32::<1>(sd(sn, vld1q_s32(ph_r.as_ptr())));
                let ni = vshlq_n_s32::<1>(sd(sn, vld1q_s32(ph_i.as_ptr())));
                let add_noise = vbicq_u32(noise_on, has_sine);
                let add_r = vbslq_s32(has_sine, vmulq_s32(s, sr), vbslq_s32(add_noise, nr, zero));
                let add_i = vbslq_s32(has_sine, vmulq_s32(s, si), vbslq_s32(add_noise, ni, zero));
                vst1q_s32(re.as_mut_ptr().add(k), vaddq_s32(sig_r, add_r));
                vst1q_s32(im.as_mut_ptr().add(k), vaddq_s32(sig_i, add_i));
                k += 4;
            }
        }
    }
}
