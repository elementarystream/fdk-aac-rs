// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libSBRdec/src/sbrdec_freq_sca.h, libSBRdec/src/sbrdec_freq_sca.cpp.

//! Port of `sbrdec_freq_sca.h` and `sbrdec_freq_sca.cpp`: the frequency scale calculation (the
//! master frequency table and the frequency band tables of the header).
//!
//! The arithmetic is `UCHAR`/`FIXP_SGL` arithmetic in `int`, truncated where the C code stores
//! into the narrow types.
//!
//! Deviation: the port writes the band widths of the second region into a larger array than the
//! 50 entries of the C array and ignores writes past `v_k_master[57]`; for band counts beyond
//! the C arrays the reference has no defined result.

use crate::env_extr::*;
use crate::lpp_tran::{
    MAX_FREQ_COEFFS, MAX_FREQ_COEFFS_DUAL_RATE, MAX_FREQ_COEFFS_FS44100, MAX_FREQ_COEFFS_FS48000,
    MAX_FREQ_COEFFS_QUAD_RATE, MAX_NOISE_COEFFS,
};
use crate::sbr_rom::*;
use crate::sbrdecoder::{SbrError, SBRDEC_OK, SBRDEC_UNSUPPORTED_CONFIG};
use crate::transcendent::fdk_get_num_octaves_div8;
use fdk_aac_rs_fdk::common_fix::*;
use fdk_aac_rs_fdk::fdk_assert;

/// `MAX_OCTAVE`
const MAX_OCTAVE: usize = 29;
/// `MAX_SECOND_REGION`
const MAX_SECOND_REGION: usize = 50;
/// The size of the port's band width buffer (see the module documentation): the C array has
/// `MAX_OCTAVE + MAX_SECOND_REGION` entries; band counts are `UCHAR` values.
const DIFF_TOT_SIZE: usize = MAX_OCTAVE + 256;

/// `SBR_RATE`
pub type SbrRate = i32;
/// `DUAL`
pub const DUAL: SbrRate = 0;
/// `QUAD`
pub const QUAD: SbrRate = 1;

/// `getStartBand(fs, startFreq, headerDataFlags)`: the QMF band where the SBR range starts (255
/// for an unsupported sampling rate).
pub fn get_start_band(fs: u32, start_freq: u8, header_data_flags: u32) -> u8 {
    let mut fs_mapped = fs;
    let mut rate = DUAL;

    if header_data_flags & (SBRDEC_SYNTAX_USAC | SBRDEC_SYNTAX_RSVD50) != 0 {
        if header_data_flags & SBRDEC_QUAD_RATE != 0 {
            rate = QUAD;
        }
        fs_mapped = sbrdec_map_to_std_sample_rate(fs, 1);
    }

    fdk_assert!(2 * (rate + 1) <= 4);

    // startFreq is 4 bits (0..=15)
    let sf = start_freq as usize & 15;
    let r = rate as usize;
    match fs_mapped {
        192000 => FDK_SBR_DECODER_SBR_START_FREQ_192[sf],
        176400 => FDK_SBR_DECODER_SBR_START_FREQ_176[sf],
        128000 => FDK_SBR_DECODER_SBR_START_FREQ_128[sf],
        96000 | 88200 => FDK_SBR_DECODER_SBR_START_FREQ_88[r][sf],
        64000 => FDK_SBR_DECODER_SBR_START_FREQ_64[r][sf],
        48000 => FDK_SBR_DECODER_SBR_START_FREQ_48[r][sf],
        44100 => FDK_SBR_DECODER_SBR_START_FREQ_44[r][sf],
        40000 => FDK_SBR_DECODER_SBR_START_FREQ_40[r][sf],
        32000 => FDK_SBR_DECODER_SBR_START_FREQ_32[r][sf],
        24000 => FDK_SBR_DECODER_SBR_START_FREQ_24[r][sf],
        22050 => FDK_SBR_DECODER_SBR_START_FREQ_22[r][sf],
        16000 => FDK_SBR_DECODER_SBR_START_FREQ_16[r][sf],
        _ => 255,
    }
}

/// `getStopBand(fs, stopFreq, headerDataFlags, k0)`: the QMF band where the SBR range ends (255
/// for an invalid range).
pub fn get_stop_band(fs: u32, stop_freq: u8, header_data_flags: u32, k0: u8) -> u8 {
    let mut k2: u8;

    if stop_freq < 14 {
        let mut num: i32 = 2 * 64;
        let mut diff_tot = [0u8; DIFF_TOT_SIZE];

        if header_data_flags & SBRDEC_QUAD_RATE != 0 {
            num >>= 1;
        }

        // (INT * INT) / UINT: unsigned division. fs is the SBR processing rate (at least 4000);
        // guarded: a rate of 0 gives 0.
        let div = |x: i32| -> i32 { (x as u32).checked_div(fs).unwrap_or(0) as i32 };
        let mut stop_min = if fs < 32000 {
            (div(2 * 6000 * num) + 1) >> 1
        } else if fs < 64000 {
            (div(2 * 8000 * num) + 1) >> 1
        } else {
            (div(2 * 10000 * num) + 1) >> 1
        };

        stop_min = f_min(stop_min, 64);

        // Choose a stop band between k1 and 64 depending on stopFreq (0..13), based on a
        // logarithmic scale. The vectors diff0 and diff1 are used temporarily here.
        let (diff0, diff1) = diff_tot.split_at_mut(MAX_OCTAVE);
        calc_bands(diff0, stop_min as u8, 64, 13);
        shellsort(diff0, 13);
        cum_sum(stop_min as u8, diff0, 13, diff1);
        k2 = diff1[stop_freq as usize];
    } else if stop_freq == 14 {
        k2 = (2 * k0 as i32) as u8;
    } else {
        k2 = (3 * k0 as i32) as u8;
    }

    // Limit to Nyquist
    if k2 > 64 {
        k2 = 64;
    }

    // Range checks
    // 1 <= difference <= 48; 1 <= fs <= 96000
    {
        let max_freq_coeffs = if header_data_flags & SBRDEC_QUAD_RATE != 0 {
            MAX_FREQ_COEFFS_QUAD_RATE
        } else {
            MAX_FREQ_COEFFS
        } as i32;
        if (k2 as i32 - k0 as i32) > max_freq_coeffs || k2 <= k0 {
            return 255;
        }
    }

    if header_data_flags & SBRDEC_QUAD_RATE != 0 {
        // skip other checks: (k2 - k0) must be <= MAX_FREQ_COEFFS_QUAD_RATE for all fs
        return k2;
    }
    let d = k2 as i32 - k0 as i32;
    if header_data_flags & (SBRDEC_SYNTAX_USAC | SBRDEC_SYNTAX_RSVD50) != 0 {
        // 1 <= difference <= 35; 42000 <= fs <= 96000
        if fs >= 42000 && d > MAX_FREQ_COEFFS_FS44100 {
            return 255;
        }
        // 1 <= difference <= 32; 46009 <= fs <= 96000
        if fs >= 46009 && d > MAX_FREQ_COEFFS_FS48000 {
            return 255;
        }
    } else {
        // 1 <= difference <= 35; fs == 44100
        if fs == 44100 && d > MAX_FREQ_COEFFS_FS44100 {
            return 255;
        }
        // 1 <= difference <= 32; 48000 <= fs <= 96000
        if fs >= 48000 && d > MAX_FREQ_COEFFS_FS48000 {
            return 255;
        }
    }

    k2
}

/// `sbrdecUpdateFreqScale(v_k_master, numMaster, fs, hHeaderData, flags)`: generates the master
/// frequency table (IEC 14496-3 4.6.18.3.2.1).
pub fn sbrdec_update_freq_scale(
    v_k_master: &mut [u8],
    num_master: &mut u8,
    fs: u32,
    h_header_data: &SbrHeaderData,
    flags: u32,
) -> SbrError {
    let mut bpo_div16: FixpSgl; // bands_per_octave divided by 16
    let num_bands0: u8;
    let num_bands1: u8;
    let mut diff_tot = [0u8; DIFF_TOT_SIZE];
    let mut fs = fs;

    // Determine start band
    if flags & SBRDEC_QUAD_RATE != 0 {
        fs >>= 1;
    }

    let k0 = get_start_band(fs, h_header_data.bs_data.start_freq, flags);
    if k0 == 255 {
        return SBRDEC_UNSUPPORTED_CONFIG;
    }

    // Determine stop band
    let k2 = get_stop_band(fs, h_header_data.bs_data.stop_freq, flags, k0);
    if k2 == 255 {
        return SBRDEC_UNSUPPORTED_CONFIG;
    }

    if h_header_data.bs_data.freq_scale > 0 {
        // Bark
        let k1: i32;

        // FL2FXCONST_SGL(12.0f / 16.0f) etc.: the quotients are exact in float.
        bpo_div16 = match h_header_data.bs_data.freq_scale {
            1 => fl2fxconst_sgl((12.0f32 / 16.0f32) as f64),
            2 => fl2fxconst_sgl((10.0f32 / 16.0f32) as f64),
            _ => fl2fxconst_sgl((8.0f32 / 16.0f32) as f64),
        };

        // Ref: ISO/IEC 23003-3, Figure 12 - Flowchart calculation of fMaster for 4:1 system when
        // bs_freq_scale > 0
        if flags & SBRDEC_QUAD_RATE != 0 && (k0 as i16) < (bpo_div16 >> ((FRACT_BITS - 1) - 4)) {
            // bpo_div16 = floor(k0/2)*2
            bpo_div16 = (((k0 & 0xfe) as i32) << ((FRACT_BITS - 1) - 4)) as FixpSgl;
        }

        if 1000 * k2 as i32 > 2245 * k0 as i32 {
            // Two or more regions
            k1 = 2 * k0 as i32;

            num_bands0 = number_of_bands(bpo_div16, k0 as i32, k1, 0) as u8;
            num_bands1 =
                number_of_bands(bpo_div16, k1, k2 as i32, h_header_data.bs_data.alter_scale as i32)
                    as u8;
            if num_bands0 < 1 {
                return SBRDEC_UNSUPPORTED_CONFIG;
            }
            if num_bands1 < 1 {
                return SBRDEC_UNSUPPORTED_CONFIG;
            }

            let (diff0, diff1) = diff_tot.split_at_mut(MAX_OCTAVE);
            calc_bands(diff0, k0, k1 as u8, num_bands0);
            shellsort(diff0, num_bands0);
            if diff0[0] == 0 {
                return SBRDEC_UNSUPPORTED_CONFIG;
            }

            cum_sum(k0, diff0, num_bands0, v_k_master);

            calc_bands(diff1, k1 as u8, k2, num_bands1);
            shellsort(diff1, num_bands1);
            if diff0[num_bands0 as usize - 1] > diff1[0] {
                let err = modify_bands(diff0[num_bands0 as usize - 1], diff1, num_bands1);
                if err != SBRDEC_OK {
                    return SBRDEC_UNSUPPORTED_CONFIG;
                }
            }

            // Add 2nd region
            let off = (num_bands0 as usize).min(v_k_master.len());
            cum_sum(k1 as u8, diff1, num_bands1, &mut v_k_master[off..]);
            *num_master = num_bands0.wrapping_add(num_bands1); // Output nr of bands
        } else {
            // Only one region
            k1 = k2 as i32;

            num_bands0 = number_of_bands(bpo_div16, k0 as i32, k1, 0) as u8;
            if num_bands0 < 1 {
                return SBRDEC_UNSUPPORTED_CONFIG;
            }
            calc_bands(&mut diff_tot, k0, k1 as u8, num_bands0);
            shellsort(&mut diff_tot, num_bands0);
            if diff_tot[0] == 0 {
                return SBRDEC_UNSUPPORTED_CONFIG;
            }

            cum_sum(k0, &diff_tot, num_bands0, v_k_master);
            *num_master = num_bands0; // Output nr of bands
        }
    } else {
        // Linear mode
        let dk: i32;
        if h_header_data.bs_data.alter_scale == 0 {
            dk = 1;
            // FLOOR to get to few number of bands (next lower even number)
            num_bands0 = ((k2 as i32 - k0 as i32) & 254) as u8;
        } else {
            dk = 2;
            // ROUND to the closest fit
            num_bands0 = ((((k2 as i32 - k0 as i32) >> 1) + 1) & 254) as u8;
        }

        if num_bands0 < 1 {
            return SBRDEC_UNSUPPORTED_CONFIG;
            // We must return already here because 'i' can become negative below.
        }

        let k2_achived = k0 as i32 + num_bands0 as i32 * dk;
        let mut k2_diff = k2 as i32 - k2_achived;

        for i in 0..num_bands0 as usize {
            diff_tot[i] = dk as u8;
        }

        let mut incr = 0i32;
        let mut i: u8 = 0;

        // If linear scale wasn't achieved and we got too wide SBR area
        if k2_diff < 0 {
            incr = 1;
            i = 0;
        }

        // If linear scale wasn't achieved and we got too small SBR area
        if k2_diff > 0 {
            incr = -1;
            i = num_bands0 - 1;
        }

        // Adjust diff vector to get sepc. SBR range
        while k2_diff != 0 {
            diff_tot[i as usize] = (diff_tot[i as usize] as i32 - incr) as u8;
            i = (i as i32 + incr) as u8;
            k2_diff += incr;
        }

        cum_sum(k0, &diff_tot, num_bands0, v_k_master); // cumsum
        *num_master = num_bands0; // Output nr of bands
    }

    if *num_master < 1 {
        return SBRDEC_UNSUPPORTED_CONFIG;
    }

    // Ref: ISO/IEC 23003-3 Cor.3, "In 7.5.5.2, add to the requirements:"
    if flags & SBRDEC_QUAD_RATE != 0 {
        let at = |i: usize| v_k_master.get(i).copied().unwrap_or(0) as i32;
        for k in 1..*num_master as usize {
            if at(k) - at(k - 1) > k0 as i32 - 2 {
                return SBRDEC_UNSUPPORTED_CONFIG;
            }
        }
    }

    SBRDEC_OK
}

/// `calcFactorPerBand(k_start, k_stop, num_bands)`: the num_band-th root of k_start/k_stop.
pub fn calc_factor_per_band(k_start: i32, k_stop: i32, num_bands: i32) -> FixpSgl {
    // Scaled bandfactor and step 1 bit right to avoid overflow use double data type
    let mut bandfactor: FixpDbl = fl2fxconst_dbl(0.25f32 as f64); // Start value
    let mut step: FixpDbl = fl2fxconst_dbl(0.125f32 as f64); // Initial increment for factor

    let mut direction = 1;

    // Because saturation can't be done in INT IIS, changed start and stop data type from
    // FIXP_SGL to FIXP_DBL
    let start: FixpDbl = k_start << (DFRACT_BITS - 8);
    let stop: FixpDbl = k_stop << (DFRACT_BITS - 8);

    let mut i = 0;

    while step > 0 {
        i += 1;
        let mut temp = stop;

        // Calculate temp^num_bands:
        for _ in 0..num_bands {
            // temp = fMult(temp,bandfactor);
            temp = f_mult_div2(temp, bandfactor) << 2;
        }

        if temp < start {
            // Factor too strong, make it weaker
            if direction == 0 {
                // Halfen step. Right shift is not done as fract because otherwise the lowest bit
                // cannot be cleared due to rounding
                step >>= 1;
            }
            direction = 1;
            bandfactor = bandfactor.wrapping_add(step);
        } else {
            // Factor is too weak: make it stronger
            if direction == 1 {
                step >>= 1;
            }
            direction = 0;
            bandfactor = bandfactor.wrapping_sub(step);
        }

        if i > 100 {
            step = 0;
        }
    }
    if bandfactor >= fl2fxconst_dbl(0.5) {
        MAXVAL_SGL as FixpSgl
    } else {
        fx_dbl2fx_sgl(bandfactor << 1)
    }
}

/// `numberOfBands(bpo_div16, start, stop, warpFlag)`: the number of SBR bands between the start
/// and stop band for the given bands per octave (divided by 16); with the warp flag the band
/// density is decreased by a factor of 1/1.3.
pub fn number_of_bands(bpo_div16: FixpSgl, start: i32, stop: i32, warp_flag: i32) -> i32 {
    let mut num_bands_div128 =
        fx_dbl2fx_sgl(f_mult(fdk_get_num_octaves_div8(start, stop), bpo_div16));

    if warp_flag != 0 {
        // Apply the warp factor of 1.3 to get wider bands.  We use a value of 32768/25200
        // instead of the exact value to avoid critical cases of rounding.
        num_bands_div128 =
            fx_dbl2fx_sgl(f_mult(num_bands_div128, fl2fxconst_sgl(25200.0 / 32768.0)));
    }

    // add scaled 1 for rounding to even numbers:
    num_bands_div128 =
        (num_bands_div128 as i32 + fl2fxconst_sgl((1.0f32 / 128.0f32) as f64) as i32) as FixpSgl;
    // scale back to right aligned integer and double the value:
    2 * ((num_bands_div128 as i32) >> (FRACT_BITS - 7))
}

/// `CalcBands(diff, start, stop, num_bands)`: the width of each SBR band in QMF channels (the
/// bands get wider from start to stop, bark scale).
pub fn calc_bands(diff: &mut [u8], start: u8, stop: u8, num_bands: u8) {
    let bandfactor = calc_factor_per_band(start as i32, stop as i32, num_bands as i32);

    let mut previous = stop as i32; // Start with highest QMF channel
                                    // Shift left to gain some accuracy
    let mut exact: FixpSgl = ((stop as i32) << (FRACT_BITS - 8)) as FixpSgl;

    for i in (0..num_bands as usize).rev() {
        // Calculate border of next lower sbr band
        exact = fx_dbl2fx_sgl(f_mult(exact, bandfactor));

        // Add scaled 0.5 for rounding: We use a value 128/256 instead of 0.5 to avoid some
        // critical cases of rounding.
        let temp = (exact as i32 + fl2fxconst_sgl(128.0 / 32768.0) as i32) as FixpSgl;

        // scale back to right alinged integer:
        let current = (temp as i32) >> (FRACT_BITS - 8);

        // Save width of band i
        if let Some(d) = diff.get_mut(i) {
            *d = (previous - current) as u8;
        }
        previous = current;
    }
}

/// `cumSum(start_value, diff, length, start_adress)`: the cumulated sum vector of the delta
/// vector (`length + 1` values; writes past the destination are ignored, see the module
/// documentation).
pub fn cum_sum(start_value: u8, diff: &[u8], length: u8, start_adress: &mut [u8]) {
    let mut prev = start_value;
    if let Some(d) = start_adress.get_mut(0) {
        *d = start_value;
    }
    for i in 1..=length as usize {
        prev = prev.wrapping_add(diff[i - 1]);
        if let Some(d) = start_adress.get_mut(i) {
            *d = prev;
        }
    }
}

/// `modifyBands(max_band_previous, diff, length)`: adapts the width of the bands of the second
/// region so that it does not start with a band smaller than the highest band of the first.
pub fn modify_bands(max_band_previous: u8, diff: &mut [u8], length: u8) -> SbrError {
    let length = length as usize;
    let mut change = max_band_previous as i32 - diff[0] as i32;

    // Limit the change so that the last band cannot get narrower than the first one
    if change > (diff[length - 1] as i32 - diff[0] as i32) >> 1 {
        change = (diff[length - 1] as i32 - diff[0] as i32) >> 1;
    }

    diff[0] = (diff[0] as i32 + change) as u8;
    diff[length - 1] = (diff[length - 1] as i32 - change) as u8;
    shellsort(diff, length as u8);

    SBRDEC_OK
}

/// `sbrdecUpdateHiRes(h_hires, num_hires, v_k_master, num_bands, xover_band)`: the high
/// resolution frequency band table.
pub fn sbrdec_update_hi_res(
    h_hires: &mut [u8],
    num_hires: &mut u8,
    v_k_master: &[u8],
    num_bands: u8,
    xover_band: u8,
) {
    *num_hires = num_bands.wrapping_sub(xover_band);

    for i in xover_band as usize..=num_bands as usize {
        h_hires[i - xover_band as usize] = v_k_master[i];
    }
}

/// `sbrdecUpdateLoRes(h_lores, num_lores, h_hires, num_hires)`: the low resolution table from
/// the high resolution table.
pub fn sbrdec_update_lo_res(h_lores: &mut [u8], num_lores: &mut u8, h_hires: &[u8], num_hires: u8) {
    if num_hires & 1 == 0 {
        // If even number of hires bands
        *num_lores = num_hires >> 1;
        // Use every second lores=hires[0,2,4...]
        for i in 0..=*num_lores as usize {
            h_lores[i] = h_hires[i * 2];
        }
    } else {
        // Odd number of hires, which means xover is odd
        *num_lores = ((num_hires as i32 + 1) >> 1) as u8;
        // Use lores=hires[0,1,3,5 ...]
        h_lores[0] = h_hires[0];
        for i in 1..=*num_lores as usize {
            h_lores[i] = h_hires[i * 2 - 1];
        }
    }
}

/// `sbrdecDownSampleLoRes(v_result, num_result, freqBandTableRef, num_Ref)`: derives a low
/// resolution frequency table (`num_result + 1` entries) from a reference table.
pub fn sbrdec_down_sample_lo_res(
    v_result: &mut [u8],
    num_result: u8,
    freq_band_table_ref: &[u8],
    num_ref: u8,
) {
    let mut v_index = [0i32; MAX_FREQ_COEFFS >> 1];

    // init
    let mut org_length = num_ref as i32;
    let mut result_length = num_result as i32;

    v_index[0] = 0; // Always use left border
    let mut i = 0usize;
    // DEVIATION: the port stops at the end of v_index[] and at a zero divisor, where the
    // reference has no defined result.
    while org_length > 0 && result_length > 0 && i + 1 < v_index.len() {
        // Create downsample vector
        i += 1;
        let step = org_length / result_length;
        org_length -= step;
        result_length -= 1;
        v_index[i] = v_index[i - 1] + step;
    }

    for j in 0..=i {
        // Use downsample vector to index LoResolution vector
        if let Some(d) = v_result.get_mut(j) {
            *d = freq_band_table_ref.get(v_index[j] as usize).copied().unwrap_or(0);
        }
    }
}

/// `shellsort(in, n)`: sorts the first `n` values in ascending order.
pub fn shellsort(input: &mut [u8], n: u8) {
    let n = n as i32;
    let mut inc = 1i32;

    loop {
        inc = 3 * inc + 1;
        if inc > n {
            break;
        }
    }

    loop {
        inc /= 3;
        for i in inc..n {
            let v = input[i as usize];
            let mut j = i;
            loop {
                let w = input[(j - inc) as usize];
                if w <= v {
                    break;
                }
                input[j as usize] = w;
                j -= inc;
                if j < inc {
                    break;
                }
            }
            input[j as usize] = v;
        }
        if inc <= 1 {
            break;
        }
    }
}

/// `resetFreqBandTables(hHeaderData, flags)`: recalculates the frequency band tables of the
/// header.
pub fn reset_freq_band_tables(h_header_data: &mut SbrHeaderData, flags: u32) -> SbrError {
    let mut n_bands_lo = 0u8;
    let mut n_bands_hi = 0u8;

    // Calculate master frequency function
    let mut v_k_master = h_header_data.freq_band_data.v_k_master;
    let mut num_master = h_header_data.freq_band_data.num_master;
    let err = sbrdec_update_freq_scale(
        &mut v_k_master,
        &mut num_master,
        h_header_data.sbr_proc_smpl_rate,
        h_header_data,
        flags,
    );
    let h_freq = &mut h_header_data.freq_band_data;
    h_freq.v_k_master = v_k_master;
    h_freq.num_master = num_master;

    if err != SBRDEC_OK || h_header_data.bs_info.xover_band > h_freq.num_master {
        return SBRDEC_UNSUPPORTED_CONFIG;
    }

    // Derive Hiresolution from master frequency function
    let xover_band = h_header_data.bs_info.xover_band;
    sbrdec_update_hi_res(
        &mut h_freq.freq_band_table_hi,
        &mut n_bands_hi,
        &h_freq.v_k_master,
        h_freq.num_master,
        xover_band,
    );
    // Derive Loresolution from Hiresolution
    sbrdec_update_lo_res(
        &mut h_freq.freq_band_table_lo,
        &mut n_bands_lo,
        &h_freq.freq_band_table_hi,
        n_bands_hi,
    );

    // Check index to freqBandTable[0]
    let max_lo = (if h_header_data.number_of_analysis_bands == 16 {
        MAX_FREQ_COEFFS_QUAD_RATE
    } else {
        MAX_FREQ_COEFFS_DUAL_RATE
    } >> 1) as u8;
    if n_bands_lo == 0 || n_bands_lo > max_lo {
        return SBRDEC_UNSUPPORTED_CONFIG;
    }

    h_freq.n_sfb[0] = n_bands_lo;
    h_freq.n_sfb[1] = n_bands_hi;

    let lsb = h_freq.freq_band_table_lo[0] as i32;
    let usb = h_freq.freq_band_table_lo[n_bands_lo as usize] as i32;

    // Check for start frequency border k_x:
    // - ISO/IEC 14496-3 4.6.18.3.6 Requirements
    // - ISO/IEC 23003-3 7.5.5.2    Modifications and additions to the MPEG-4 SBR tool
    // Note that lsb > as hHeaderData->numberOfAnalysisBands is a valid SBR config for 24 band
    // QMF analysis.
    if lsb > (if flags & SBRDEC_QUAD_RATE != 0 { 16 } else { 32 }) || lsb >= usb {
        return SBRDEC_UNSUPPORTED_CONFIG;
    }

    // Calculate number of noise bands

    let k2 = h_freq.freq_band_table_hi[n_bands_hi as usize] as i32;
    let kx = h_freq.freq_band_table_hi[0] as i32;

    if h_header_data.bs_data.noise_bands == 0 {
        h_freq.n_nfb = 1;
    } else {
        // Calculate no of noise bands 1,2 or 3 bands/octave
        // Fetch number of octaves divided by 32
        let mut int_temp = (fdk_get_num_octaves_div8(kx, k2) as i32) >> 2;

        // Integer-Multiplication with number of bands:
        int_temp *= h_header_data.bs_data.noise_bands as i32;

        // Add scaled 0.5 for rounding:
        int_temp += fl2fxconst_sgl((0.5f32 / 32.0f32) as f64) as i32;

        // Convert to right-aligned integer:
        int_temp >>= FRACT_BITS - 1 /*sign*/ - 5 /* rescale */;

        if int_temp == 0 {
            int_temp = 1;
        }

        if int_temp > MAX_NOISE_COEFFS as i32 {
            return SBRDEC_UNSUPPORTED_CONFIG;
        }

        h_freq.n_nfb = int_temp as u8;
    }

    h_freq.n_invf_bands = h_freq.n_nfb;

    // Get noise bands
    let lo = h_freq.freq_band_table_lo;
    sbrdec_down_sample_lo_res(&mut h_freq.freq_band_table_noise, h_freq.n_nfb, &lo, n_bands_lo);

    // save old highband; required for overlap in usac when headerchange occurs at XVAR and VARX
    // frame;
    h_freq.ov_high_subband = h_freq.high_subband;

    h_freq.low_subband = lsb as u8;
    h_freq.high_subband = usb as u8;

    SBRDEC_OK
}
