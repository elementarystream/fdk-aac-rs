// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libSBRenc/src/sbrenc_freq_sca.h, libSBRenc/src/sbrenc_freq_sca.cpp.

//! Port of `sbrenc_freq_sca.h` and `sbrenc_freq_sca.cpp`: the SBR frequency band tables (master
//! table, high and low resolution tables, start and stop bands).

use fdk_aac_rs_fdk::common_fix::*;
use fdk_aac_rs_fdk::fixpoint_math::{calc_ld_int, f_div_norm, f_pow, LD_DATA_SHIFT};
use fdk_aac_rs_fdk::scale::scale_value;

use crate::sbr_def::{MAX_FREQ_COEFFS, MAX_FREQ_COEFFS_FS44100, MAX_FREQ_COEFFS_FS48000};
use crate::sbr_misc::fdk_sbr_enc_shellsort_int;

mod tables;
pub use tables::*;

/// `MAX_OCTAVE`
pub const MAX_OCTAVE: usize = 29;
/// `MAX_SECOND_REGION`
pub const MAX_SECOND_REGION: usize = 50;

/// `FDKsbrEnc_getSbrStartFreqRAW(startFreq, fsCore)`: the start frequency in Hz, -1 for an
/// invalid index.
pub fn fdk_sbr_enc_get_sbr_start_freq_raw(start_freq: i32, fs_core: i32) -> i32 {
    if !(0..=15).contains(&start_freq) {
        return -1;
    }
    // Update startFreq struct
    let mut result = get_start_freq(fs_core, start_freq);

    result = (result * (fs_core >> 5) + 1) >> 1; // (result*fsSBR/QMFbands+1)>>1;

    result
}

/// `FDKsbrEnc_getSbrStopFreqRAW(stopFreq, fsCore)`: the stop frequency in Hz, -1 for an
/// invalid index.
pub fn fdk_sbr_enc_get_sbr_stop_freq_raw(stop_freq: i32, fs_core: i32) -> i32 {
    if !(0..=13).contains(&stop_freq) {
        return -1;
    }

    // Uppdate stopFreq struct
    let mut result = get_stop_freq(fs_core, stop_freq);
    result = (result * (fs_core >> 5) + 1) >> 1; // (result*fsSBR/QMFbands+1)>>1;

    result
}

/// `getStartFreq(fsCore, start_freq)`
///
/// Bounded: indices outside the offset table give an offset of 0.
fn get_start_freq(fs_core: i32, start_freq: i32) -> i32 {
    let k0_min = match fs_core {
        8000 => 24,  // (3000 * nQmfChannels / fsSBR ) + 0.5
        11025 => 17, // (3000 * nQmfChannels / fsSBR ) + 0.5
        12000 => 16, // (3000 * nQmfChannels / fsSBR ) + 0.5
        16000 => 16, // (4000 * nQmfChannels / fsSBR ) + 0.5
        22050 => 12, // (4000 * nQmfChannels / fsSBR ) + 0.5
        24000 => 11, // (4000 * nQmfChannels / fsSBR ) + 0.5
        32000 => 10, // (5000 * nQmfChannels / fsSBR ) + 0.5
        44100 => 7,  // (5000 * nQmfChannels / fsSBR ) + 0.5
        48000 => 7,  // (5000 * nQmfChannels / fsSBR ) + 0.5
        96000 => 3,  // (5000 * nQmfChannels / fsSBR ) + 0.5
        _ => 11,     // illegal fs
    };

    let v_offset: &[i32; 16] = match fs_core {
        8000 => &V_OFFSET_0,
        11025 => &V_OFFSET_1,
        12000 => &V_OFFSET_2,
        16000 => &V_OFFSET_3,
        22050 | 24000 | 32000 => &V_OFFSET_4,
        44100 | 48000 | 96000 => &V_OFFSET_5,
        _ => &V_OFFSET_6,
    };
    k0_min + v_offset.get(start_freq as usize).copied().unwrap_or(0)
}

/// `getStopFreq(fsCore, stop_freq)`
///
/// Deviation, where the reference has no defined result: the port uses band width changes of 0
/// (the result is `k1_min`) or adds 0.
fn get_stop_freq(fs_core: i32, stop_freq: i32) -> i32 {
    let mut v_dstop = [0i32; 13];

    let (k1_min, v_stop_freq): (i32, Option<&[i32; 14]>) = match fs_core {
        8000 => (48, Some(&V_STOP_FREQ_16)),
        11025 => (35, Some(&V_STOP_FREQ_22)),
        12000 => (32, Some(&V_STOP_FREQ_24)),
        16000 => (32, Some(&V_STOP_FREQ_32)),
        22050 => (23, Some(&V_STOP_FREQ_44)),
        24000 => (21, Some(&V_STOP_FREQ_48)),
        32000 => (20, Some(&V_STOP_FREQ_64)),
        44100 => (15, Some(&V_STOP_FREQ_88)),
        48000 => (13, Some(&V_STOP_FREQ_96)),
        96000 => (7, Some(&V_STOP_FREQ_192)),
        _ => (21, None), // illegal fs
    };

    // Ensure increasing bandwidth
    if let Some(v) = v_stop_freq {
        for i in 0..=12 {
            v_dstop[i] = v[i + 1] - v[i];
        }
    }

    fdk_sbr_enc_shellsort_int(&mut v_dstop, 13); // Sort bandwidth changes

    let mut result = k1_min;
    for i in 0..stop_freq.max(0) as usize {
        result += v_dstop.get(i).copied().unwrap_or(0);
    }

    result
}

/// `FDKsbrEnc_FindStartAndStopBand(srSbr, srCore, noChannels, startFreq, stopFreq, k0, k2)`:
/// returns 0 if OK.
pub fn fdk_sbr_enc_find_start_and_stop_band(
    sr_sbr: i32,
    sr_core: i32,
    no_channels: i32,
    start_freq: i32,
    stop_freq: i32,
    k0: &mut i32,
    k2: &mut i32,
) -> i32 {
    // Update startFreq struct
    *k0 = get_start_freq(sr_core, start_freq);

    // Test if start freq is outside corecoder range
    if sr_sbr.wrapping_mul(no_channels) < (*k0).wrapping_mul(sr_core) {
        // raise the cross-over frequency and/or lower the number of target bands per octave (or
        // lower the sampling frequency)
        return 1;
    }

    // Update stopFreq struct
    if stop_freq < 14 {
        *k2 = get_stop_freq(sr_core, stop_freq);
    } else if stop_freq == 14 {
        *k2 = 2 * *k0;
    } else {
        *k2 = 3 * *k0;
    }

    // limit to Nyqvist
    if *k2 > no_channels {
        *k2 = no_channels;
    }

    // Test for invalid  k0 k2 combinations
    if sr_core == 22050 && (*k2 - *k0) > MAX_FREQ_COEFFS_FS44100 {
        return 1; // Number of bands exceeds valid range of MAX_FREQ_COEFFS for fs=44.1kHz
    }

    if sr_core >= 24000 && (*k2 - *k0) > MAX_FREQ_COEFFS_FS48000 {
        return 1; // Number of bands exceeds valid range of MAX_FREQ_COEFFS for fs>=48kHz
    }

    if (*k2 - *k0) > MAX_FREQ_COEFFS as i32 {
        return 1; // Number of bands exceeds valid range of MAX_FREQ_COEFFS
    }

    if (*k2 - *k0) < 0 {
        return 1; // Number of bands is negative
    }

    0
}

/// `FDKsbrEnc_UpdateFreqScale(v_k_master, h_num_bands, k0, k2, freqScale, alterScale)`:
/// computes the master frequency table. Returns 0 if OK.
pub fn fdk_sbr_enc_update_freq_scale(
    v_k_master: &mut [u8],
    h_num_bands: &mut i32,
    k0: i32,
    k2: i32,
    freq_scale: i32,
    alter_scale: i32,
) -> i32 {
    let mut b_p_o = 0; // bands_per_octave
                       // Internal variables
    let mut diff_tot = [0i32; MAX_OCTAVE + MAX_SECOND_REGION];

    // Init
    if freq_scale == 1 {
        b_p_o = 12;
    }
    if freq_scale == 2 {
        b_p_o = 10;
    }
    if freq_scale == 3 {
        b_p_o = 8;
    }

    if freq_scale > 0 {
        // Bark
        let warp: FixpDbl = if alter_scale == 0 {
            fl2fxconst_dbl(0.5f32 as f64) // 1.0/(1.0*2.0)
        } else {
            fl2fxconst_dbl((1.0f32 / 2.6f32) as f64) // 1.0/(1.3*2.0);
        };

        if 4 * k2 >= 9 * k0 {
            // two or more regions (how many times the basis band is copied)
            let k1 = 2 * k0;

            let num_bands0 = number_of_bands(b_p_o, k0, k1, fl2fxconst_dbl(0.5f32 as f64));
            let num_bands1 = number_of_bands(b_p_o, k1, k2, warp);

            let (diff0, diff1) = diff_tot.split_at_mut(MAX_OCTAVE);

            calc_bands(diff0, k0, k1, num_bands0); // CalcBands1 => diff0
            fdk_sbr_enc_shellsort_int(diff0, num_bands0); // SortBands sort diff0

            if diff0[0] == 0 {
                // too wide FB bands for target tuning
                // raise the cross-over frequency and/or lower the number of target bands per
                // octave (or lower the sampling frequency
                return 1;
            }

            cum_sum(k0, diff0, num_bands0, v_k_master); // cumsum

            calc_bands(diff1, k1, k2, num_bands1); // CalcBands2 => diff1
            fdk_sbr_enc_shellsort_int(diff1, num_bands1); // SortBands sort diff1
            if diff0[(num_bands0 - 1) as usize] > diff1[0] {
                // max(1) > min(2)
                if modify_bands(diff0[(num_bands0 - 1) as usize], diff1, num_bands1) != 0 {
                    return 1;
                }
            }

            // Add 2'nd region
            cum_sum(k1, diff1, num_bands1, &mut v_k_master[num_bands0 as usize..]);
            *h_num_bands = num_bands0 + num_bands1; // Output nr of bands
        } else {
            // one region
            let k1 = k2;

            let num_bands0 = number_of_bands(b_p_o, k0, k1, fl2fxconst_dbl(0.5f32 as f64));
            let diff0 = &mut diff_tot[..MAX_OCTAVE];
            calc_bands(diff0, k0, k1, num_bands0); // CalcBands1 => diff0
            fdk_sbr_enc_shellsort_int(diff0, num_bands0); // SortBands sort diff0

            if diff0[0] == 0 {
                // too wide FB bands for target tuning
                return 1;
            }

            cum_sum(k0, diff0, num_bands0, v_k_master); // cumsum
            *h_num_bands = num_bands0; // Output nr of bands
        }
    } else {
        // Linear mode
        let dk;
        let num_bands0;
        if alter_scale == 0 {
            dk = 1;
            num_bands0 = 2 * ((k2 - k0) / 2); // FLOOR to get to few number of bands
        } else {
            dk = 2;
            num_bands0 = 2 * (((k2 - k0) / dk + 1) / 2); // ROUND to get closest fit
        }

        let k2_achived = k0 + num_bands0 * dk;
        let mut k2_diff = k2 - k2_achived;

        for d in diff_tot.iter_mut().take(num_bands0.max(0) as usize) {
            *d = dk;
        }

        let mut incr = 0;
        let mut i = 0i32;
        // If linear scale wasn't achived and we got wide SBR are
        if k2_diff < 0 {
            incr = 1;
            i = 0;
        }

        // If linear scale wasn't achived and we got small SBR are
        if k2_diff > 0 {
            incr = -1;
            i = num_bands0 - 1;
        }

        // Adjust diff vector to get sepc. SBR range
        while k2_diff != 0 {
            // Deviation: where the reference has no defined result, the port stops.
            let Some(d) = usize::try_from(i).ok().and_then(|i| diff_tot.get_mut(i)) else {
                break;
            };
            *d -= incr;
            i += incr;
            k2_diff += incr;
        }

        cum_sum(k0, &diff_tot, num_bands0, v_k_master); // cumsum
        *h_num_bands = num_bands0; // Output nr of bands
    }

    if *h_num_bands < 1 {
        return 1; // To small sbr area
    }

    0
}

/// `numberOfBands(b_p_o, start, stop, warp_factor)`:
/// `2 * (INT)(b_p_o * log2(stop / start) * warp_factor + 0.5)`.
fn number_of_bands(b_p_o: i32, start: i32, stop: i32, warp_factor: FixpDbl) -> i32 {
    let mut result =
        b_p_o.wrapping_mul(f_mult(calc_ld_int(stop).wrapping_sub(calc_ld_int(start)), warp_factor));
    result = result.wrapping_add(fl2fx_dbl(0.5) >> LD_DATA_SHIFT);
    // do not optimize anymore (rounding!!)
    (result >> ((DFRACT_BITS - 1) - LD_DATA_SHIFT)) << 1
}

/// `CalcBands(diff, start, stop, num_bands)`
fn calc_bands(diff: &mut [i32], start: i32, stop: i32, num_bands: i32) {
    let mut previous = start;
    for i in 1..=num_bands {
        let mut qb = 0;
        let mut qe = 0;
        let mut qtmp = 0;
        let base = f_div_norm(stop, start, &mut qb);
        let exp = f_div_norm(i, num_bands, &mut qe);
        let mut tmp = f_pow(base, qb, exp, qe, &mut qtmp);
        tmp = f_mult(tmp, start.wrapping_shl(24));
        let mut current = scale_value(tmp, qtmp - 23);
        current = (current + 1) >> 1; // rounding
        diff[(i - 1) as usize] = current - previous;
        previous = current;
    }
}

/// `cumSum(start_value, diff, length, start_adress)`
///
/// Bounded: stops at the end of the master table.
fn cum_sum(start_value: i32, diff: &[i32], length: i32, start_adress: &mut [u8]) {
    start_adress[0] = start_value as u8;
    for i in 1..=(length.max(0) as usize).min(start_adress.len() - 1) {
        start_adress[i] = (start_adress[i - 1] as i32 + diff[i - 1]) as u8;
    }
}

/// `modifyBands(max_band_previous, diff, length)`
fn modify_bands(max_band_previous: i32, diff: &mut [i32], length: i32) -> i32 {
    let mut change = max_band_previous - diff[0];

    // Limit the change so that the last band cannot get narrower than the first one
    if change > (diff[(length - 1) as usize] - diff[0]) / 2 {
        change = (diff[(length - 1) as usize] - diff[0]) / 2;
    }

    diff[0] += change;
    diff[(length - 1) as usize] -= change;
    fdk_sbr_enc_shellsort_int(diff, length);

    0
}

/// `FDKsbrEnc_UpdateHiRes(h_hires, num_hires, v_k_master, num_master, xover_band)`: the high
/// resolution table from the master table. Returns 0.
///
/// `num_master` must be at most 48 (`MAX_FREQ_COEFFS`), as `FDKsbrEnc_UpdateFreqScale()`
/// guarantees.
pub fn fdk_sbr_enc_update_hi_res(
    h_hires: &mut [u8],
    num_hires: &mut i32,
    v_k_master: &[u8],
    num_master: i32,
    xover_band: &mut i32,
) -> i32 {
    // Bounded: a crossover band outside of the master table reads 0.
    let at_xover = usize::try_from(*xover_band).ok().and_then(|i| v_k_master.get(i)).copied();
    if at_xover.unwrap_or(0) > 32 || *xover_band > num_master {
        // v_k_master[*xover_band] > noQMFChannels(dualRate)/divider
        // xover_band error, too big for this startFreq. Will be clipped

        // Calculate maximum value for xover_band
        let mut max1 = 0;
        let max2 = num_master;
        while v_k_master[(max1 + 1) as usize] < 32 && (max1 + 1) < max2 {
            // noQMFChannels(dualRate)/divider
            max1 += 1;
        }

        *xover_band = max1;
    }

    *num_hires = num_master - *xover_band;
    for i in *xover_band..=num_master {
        h_hires[(i - *xover_band) as usize] = v_k_master[i as usize];
    }

    0
}

/// `FDKsbrEnc_UpdateLoRes(h_lores, num_lores, h_hires, num_hires)`: the low resolution table
/// from the high resolution table.
pub fn fdk_sbr_enc_update_lo_res(
    h_lores: &mut [u8],
    num_lores: &mut i32,
    h_hires: &[u8],
    num_hires: i32,
) {
    if num_hires % 2 == 0 {
        // if even number of hires bands
        *num_lores = num_hires / 2;
        // Use every second lores=hires[0,2,4...]
        for i in 0..=*num_lores as usize {
            h_lores[i] = h_hires[i * 2];
        }
    } else {
        // odd number of hires which means xover is odd
        *num_lores = (num_hires + 1) / 2;
        // Use lores=hires[0,1,3,5 ...]
        h_lores[0] = h_hires[0];
        for i in 1..=*num_lores as usize {
            h_lores[i] = h_hires[i * 2 - 1];
        }
    }
}
