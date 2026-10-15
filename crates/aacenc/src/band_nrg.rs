// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACenc/src/band_nrg.h, libAACenc/src/band_nrg.cpp.

//! Port of `band_nrg.h`: band/line energy calculations.
//!
//! The `aarch64` and `x86` translation units of `band_nrg.cpp` are identical; the variants differ
//! only in the primitives.
//!
//! # Shift amounts
//!
//! The shift amounts are derived from `sfbMaxScaleSpec`, which `FDKaacEnc_CalcSfbMaxScaleSpec()`
//! limits to `0..=30` and which `FDKaacEnc_psyMain()` only lowers by at most its minimum; all
//! shifts are then within `0..=31`. The port takes shift amounts modulo 32 (the scalar shifts of
//! both reference builds). Hazard: both builds vectorize loops of
//! `FDKaacEnc_CalcBandEnergyOptimLong()` (NEON `ushl`/`sshl` on arm64; SSE on x86), whose shifts
//! give 0 for amounts of 32 or more while the remaining iterations use scalar shifts. Measured:
//! identical results for `sfbMaxScaleSpec` in `-11..=35` on both builds, differences in about a
//! third of random band sets with values in `-40..=60`. The other
//! functions of this module gave identical results in that range as well.

use fdk_aac_rs_fdk::common_fix::*;

/// `FDKaacEnc_CalcSfbMaxScaleSpec()`: the headroom of each band (at most 30).
/// The lines `lo..hi` of `x` (none for `hi <= lo`, where the C loops do not run): the band loops
/// iterate over this slice, so that the compiler can vectorize them.
#[inline(always)]
fn band<T>(x: &[T], lo: i32, hi: i32) -> &[T] {
    if lo < hi {
        &x[lo as usize..hi as usize]
    } else {
        &[]
    }
}

pub fn fdk_aac_enc_calc_sfb_max_scale_spec(
    mdct_spectrum: &[FixpDbl],
    band_offset: &[i32],
    sfb_max_scale_spec: &mut [i32],
    num_bands: i32,
) {
    for i in 0..num_bands.max(0) as usize {
        // (the maximum does not depend on the order)
        let max_spc: FixpDbl = band(mdct_spectrum, band_offset[i], band_offset[i + 1])
            .iter()
            .fold(0, |m, &x| fix_max(m, fixp_abs(x)));
        let j = cnt_leading_zeros(max_spc) - 1;
        sfb_max_scale_spec[i] = fix_min(DFRACT_BITS - 2, j);
        // CountLeadingBits() is not necessary here since test value is always > 0
    }
}

/// `FDKaacEnc_CheckBandEnergyOptim()`: band energies (scaled by the band's headroom) and their
/// ld values; returns the unscaled maximum energy.
#[allow(clippy::too_many_arguments)]
pub fn fdk_aac_enc_check_band_energy_optim(
    mdct_spectrum: &[FixpDbl],
    sfb_max_scale_spec: &[i32],
    band_offset: &[i32],
    num_bands: i32,
    band_energy: &mut [FixpDbl],
    band_energy_ld_data: &mut [FixpDbl],
    min_spec_shift: i32,
) -> FixpDbl {
    let mut nr: usize = 0;
    let mut max_nrg_ld: FixpDbl = fl2fxconst_dbl(-1.0);
    let nb = num_bands.max(0) as usize;

    // The energies first, then their ld values with the vector CalcLdData: each band's
    // energy depends only on its lines, and each ld value only on its energy.
    for i in 0..nb {
        let scale = fix_max(0, sfb_max_scale_spec[i] - 4);
        // (the wrapping sum does not depend on the order)
        let tmp: FixpDbl = band(mdct_spectrum, band_offset[i], band_offset[i + 1])
            .iter()
            .fold(0, |t, &x| f_pow2_add_div2(t, x.wrapping_shl(scale as u32)));
        band_energy[i] = tmp << 1;
    }
    // calculate ld of bandNrg
    ld_data_vector(&band_energy[..nb], band_energy_ld_data);

    for i in 0..nb {
        let scale = fix_max(0, sfb_max_scale_spec[i] - 4);
        // subtract scaling
        if band_energy_ld_data[i] != fl2fxconst_dbl(-1.0) {
            band_energy_ld_data[i] =
                band_energy_ld_data[i].wrapping_sub(scale.wrapping_mul(fl2fxconst_dbl(2.0 / 64.0)));
        }
        // find index of maxNrg
        if band_energy_ld_data[i] > max_nrg_ld {
            max_nrg_ld = band_energy_ld_data[i];
            nr = i;
        }
    }

    // return unscaled maxNrg
    let mut scale = fix_max(0, sfb_max_scale_spec[nr] - 4);
    scale = fix_max(min_spec_shift.wrapping_sub(scale).wrapping_mul(2), -(DFRACT_BITS - 1));

    scale_value(band_energy[nr], scale)
}

/// `FDKaacEnc_CalcBandEnergyOptimLong()`: band energies and their ld values of a long block,
/// scaled down so that the largest ld value is not positive; returns the number of shifts.
pub fn fdk_aac_enc_calc_band_energy_optim_long(
    mdct_spectrum: &[FixpDbl],
    sfb_max_scale_spec: &[i32],
    band_offset: &[i32],
    num_bands: i32,
    band_energy: &mut [FixpDbl],
    band_energy_ld_data: &mut [FixpDbl],
) -> i32 {
    let mut shift_bits: i32 = 0;
    let mut max_nrg_ld: FixpDbl = 0;
    let nb = num_bands.max(0) as usize;

    for i in 0..nb {
        // max sfbWidth = 96 ; 2^7=128 => 7/2 = 4 (spc*spc)
        let leading_bits: i32 = sfb_max_scale_spec[i] - 4;
        let lines = band(mdct_spectrum, band_offset[i], band_offset[i + 1]);
        // don't use scaleValue() here, it increases workload quite sufficiently...
        let tmp: FixpDbl = if leading_bits >= 0 {
            lines.iter().fold(0, |t, &x| f_pow2_add_div2(t, x.wrapping_shl(leading_bits as u32)))
        } else {
            let shift: i32 = -leading_bits;
            lines.iter().fold(0, |t, &x| f_pow2_add_div2(t, x.wrapping_shr(shift as u32)))
        };
        band_energy[i] = tmp << 1;
    }

    // calculate ld of bandNrg, subtract scaling
    ld_data_vector(&band_energy[..nb], band_energy_ld_data);
    for i in (0..nb).rev() {
        let scale_diff: FixpDbl =
            (sfb_max_scale_spec[i] - 4).wrapping_mul(fl2fxconst_dbl(2.0 / 64.0));

        band_energy_ld_data[i] = if band_energy_ld_data[i]
            >= (fl2fxconst_dbl(-1.0) >> 1).wrapping_add(scale_diff >> 1)
        {
            band_energy_ld_data[i].wrapping_sub(scale_diff)
        } else {
            fl2fxconst_dbl(-1.0)
        };
        // find maxNrgLd
        max_nrg_ld = fix_max(max_nrg_ld, band_energy_ld_data[i]);
    }

    if max_nrg_ld <= 0 {
        for i in (0..nb).rev() {
            let scale = fix_min((sfb_max_scale_spec[i] - 4).wrapping_shl(1), DFRACT_BITS - 1);
            band_energy[i] = scale_value(band_energy[i], scale.wrapping_neg());
        }
        0
    } else {
        // scale down NRGs
        while max_nrg_ld > 0 {
            max_nrg_ld -= fl2fxconst_dbl(2.0 / 64.0);
            shift_bits += 1;
        }
        for i in (0..nb).rev() {
            let scale = fix_min(
                ((sfb_max_scale_spec[i] - 4) + shift_bits).wrapping_shl(1),
                DFRACT_BITS - 1,
            );
            band_energy_ld_data[i] = band_energy_ld_data[i]
                .wrapping_sub(shift_bits.wrapping_mul(fl2fxconst_dbl(2.0 / 64.0)));
            band_energy[i] = scale_value(band_energy[i], scale.wrapping_neg());
        }
        shift_bits
    }
}

/// `FDKaacEnc_CalcBandEnergyOptimShort()`: band energies of a short window.
pub fn fdk_aac_enc_calc_band_energy_optim_short(
    mdct_spectrum: &[FixpDbl],
    sfb_max_scale_spec: &[i32],
    band_offset: &[i32],
    num_bands: i32,
    band_energy: &mut [FixpDbl],
) {
    let nb = num_bands.max(0) as usize;
    for i in 0..nb {
        // max sfbWidth = 36 ; 2^6=64 => 6/2 = 3 (spc*spc)
        let leading_bits: i32 = sfb_max_scale_spec[i] - 3;
        let tmp: FixpDbl = band(mdct_spectrum, band_offset[i], band_offset[i + 1])
            .iter()
            .fold(0, |t, &x| f_pow2_add_div2(t, scale_value(x, leading_bits)));
        band_energy[i] = tmp;
    }

    for i in 0..nb {
        // max sfbWidth = 36 ; 2^6=64 => 6/2 = 3 (spc*spc)
        let mut scale: i32 = (sfb_max_scale_spec[i] - 3).wrapping_mul(2).wrapping_sub(1);
        scale = fix_max(fix_min(scale, DFRACT_BITS - 1), -(DFRACT_BITS - 1));
        band_energy[i] = scale_value_saturate(band_energy[i], -scale);
    }
}

/// `FDKaacEnc_CalcBandNrgMSOpt()`: band energies of mid and side, optionally with their ld
/// values (`band_energy_mid_ld_data` and `band_energy_side_ld_data` are only written if
/// `calc_ld_data` is set).
#[allow(clippy::too_many_arguments)]
pub fn fdk_aac_enc_calc_band_nrg_ms_opt(
    mdct_spectrum_left: &[FixpDbl],
    mdct_spectrum_right: &[FixpDbl],
    sfb_max_scale_spec_left: &[i32],
    sfb_max_scale_spec_right: &[i32],
    band_offset: &[i32],
    num_bands: i32,
    band_energy_mid: &mut [FixpDbl],
    band_energy_side: &mut [FixpDbl],
    calc_ld_data: i32,
    band_energy_mid_ld_data: &mut [FixpDbl],
    band_energy_side_ld_data: &mut [FixpDbl],
) {
    let nb = num_bands.max(0) as usize;

    for i in 0..nb {
        let mut nrg_mid: FixpDbl = 0;
        let mut nrg_side: FixpDbl = 0;
        let mut min_scale = fix_min(sfb_max_scale_spec_left[i], sfb_max_scale_spec_right[i]) - 4;
        min_scale = fix_max(0, min_scale);

        let (lo, hi) = (band_offset[i] as usize, band_offset[i + 1] as usize);
        let (left, right) = if lo < hi {
            (&mdct_spectrum_left[lo..hi], &mdct_spectrum_right[lo..hi])
        } else {
            (&[][..], &[][..])
        };
        if min_scale > 0 {
            for (&l, &r) in left.iter().zip(right) {
                let spec_l: FixpDbl = l.wrapping_shl((min_scale - 1) as u32);
                let spec_r: FixpDbl = r.wrapping_shl((min_scale - 1) as u32);
                let specm = spec_l.wrapping_add(spec_r);
                let specs = spec_l.wrapping_sub(spec_r);
                nrg_mid = f_pow2_add_div2(nrg_mid, specm);
                nrg_side = f_pow2_add_div2(nrg_side, specs);
            }
        } else {
            for (&l, &r) in left.iter().zip(right) {
                let spec_l: FixpDbl = l >> 1;
                let spec_r: FixpDbl = r >> 1;
                let specm = spec_l.wrapping_add(spec_r);
                let specs = spec_l.wrapping_sub(spec_r);
                nrg_mid = f_pow2_add_div2(nrg_mid, specm);
                nrg_side = f_pow2_add_div2(nrg_side, specs);
            }
        }
        band_energy_mid[i] = fix_min(nrg_mid, MAXVAL_DBL >> 1) << 1;
        band_energy_side[i] = fix_min(nrg_side, MAXVAL_DBL >> 1) << 1;
    }

    if calc_ld_data != 0 {
        ld_data_vector(&band_energy_mid[..nb], band_energy_mid_ld_data);
        ld_data_vector(&band_energy_side[..nb], band_energy_side_ld_data);
    }

    for i in 0..nb {
        let min_scale = fix_min(sfb_max_scale_spec_left[i], sfb_max_scale_spec_right[i]);
        let mut scale: i32 = fix_max(0, min_scale.wrapping_sub(4).wrapping_mul(2));

        if calc_ld_data != 0 {
            // using the minimal scaling of left and right channel can cause very small
            // energies; check ldNrg before subtract scaling multiplication: fract*INT we don't
            // need fMult
            let minus: i32 = scale.wrapping_mul(fl2fxconst_dbl(1.0 / 64.0));

            if band_energy_mid_ld_data[i] != fl2fxconst_dbl(-1.0) {
                band_energy_mid_ld_data[i] = band_energy_mid_ld_data[i].wrapping_sub(minus);
            }

            if band_energy_side_ld_data[i] != fl2fxconst_dbl(-1.0) {
                band_energy_side_ld_data[i] = band_energy_side_ld_data[i].wrapping_sub(minus);
            }
        }
        scale = fix_min(scale, DFRACT_BITS - 1);
        band_energy_mid[i] >>= scale;
        band_energy_side[i] >>= scale;
    }
}

// The band loops (slices folded in any order) against the straight loops (copied from the
// straight port), on random bands, spectra of every magnitude and scale values; band_nrg.rs of
// the cref tests compares the functions with C.
#[cfg(test)]
mod tests {
    use super::*;

    fn fdk_aac_enc_calc_sfb_max_scale_spec_straight(
        mdct_spectrum: &[FixpDbl],
        band_offset: &[i32],
        sfb_max_scale_spec: &mut [i32],
        num_bands: i32,
    ) {
        for i in 0..num_bands.max(0) as usize {
            let mut max_spc: FixpDbl = 0;

            for j in band_offset[i]..band_offset[i + 1] {
                let tmp: FixpDbl = fixp_abs(mdct_spectrum[j as usize]);
                max_spc = fix_max(max_spc, tmp);
            }
            let j = cnt_leading_zeros(max_spc) - 1;
            sfb_max_scale_spec[i] = fix_min(DFRACT_BITS - 2, j);
            // CountLeadingBits() is not necessary here since test value is always > 0
        }
    }

    fn fdk_aac_enc_check_band_energy_optim_straight(
        mdct_spectrum: &[FixpDbl],
        sfb_max_scale_spec: &[i32],
        band_offset: &[i32],
        num_bands: i32,
        band_energy: &mut [FixpDbl],
        band_energy_ld_data: &mut [FixpDbl],
        min_spec_shift: i32,
    ) -> FixpDbl {
        let mut nr: usize = 0;
        let mut max_nrg_ld: FixpDbl = fl2fxconst_dbl(-1.0);

        for i in 0..num_bands.max(0) as usize {
            let scale = fix_max(0, sfb_max_scale_spec[i] - 4);
            let mut tmp: FixpDbl = 0;

            for j in band_offset[i]..band_offset[i + 1] {
                let spec: FixpDbl = mdct_spectrum[j as usize].wrapping_shl(scale as u32);
                tmp = f_pow2_add_div2(tmp, spec);
            }
            band_energy[i] = tmp << 1;

            // calculate ld of bandNrg, subtract scaling
            band_energy_ld_data[i] = calc_ld_data(band_energy[i]);
            if band_energy_ld_data[i] != fl2fxconst_dbl(-1.0) {
                band_energy_ld_data[i] = band_energy_ld_data[i]
                    .wrapping_sub(scale.wrapping_mul(fl2fxconst_dbl(2.0 / 64.0)));
            }
            // find index of maxNrg
            if band_energy_ld_data[i] > max_nrg_ld {
                max_nrg_ld = band_energy_ld_data[i];
                nr = i;
            }
        }

        // return unscaled maxNrg
        let mut scale = fix_max(0, sfb_max_scale_spec[nr] - 4);
        scale = fix_max(min_spec_shift.wrapping_sub(scale).wrapping_mul(2), -(DFRACT_BITS - 1));

        scale_value(band_energy[nr], scale)
    }

    fn fdk_aac_enc_calc_band_energy_optim_long_straight(
        mdct_spectrum: &[FixpDbl],
        sfb_max_scale_spec: &[i32],
        band_offset: &[i32],
        num_bands: i32,
        band_energy: &mut [FixpDbl],
        band_energy_ld_data: &mut [FixpDbl],
    ) -> i32 {
        let mut shift_bits: i32 = 0;
        let mut max_nrg_ld: FixpDbl = 0;
        let nb = num_bands.max(0) as usize;

        for i in 0..nb {
            // max sfbWidth = 96 ; 2^7=128 => 7/2 = 4 (spc*spc)
            let leading_bits: i32 = sfb_max_scale_spec[i] - 4;
            let mut tmp: FixpDbl = 0;
            // don't use scaleValue() here, it increases workload quite sufficiently...
            if leading_bits >= 0 {
                for j in band_offset[i]..band_offset[i + 1] {
                    let spec: FixpDbl = mdct_spectrum[j as usize].wrapping_shl(leading_bits as u32);
                    tmp = f_pow2_add_div2(tmp, spec);
                }
            } else {
                let shift: i32 = -leading_bits;
                for j in band_offset[i]..band_offset[i + 1] {
                    let spec: FixpDbl = mdct_spectrum[j as usize].wrapping_shr(shift as u32);
                    tmp = f_pow2_add_div2(tmp, spec);
                }
            }
            band_energy[i] = tmp << 1;
        }

        // calculate ld of bandNrg, subtract scaling
        ld_data_vector(&band_energy[..nb], band_energy_ld_data);
        for i in (0..nb).rev() {
            let scale_diff: FixpDbl =
                (sfb_max_scale_spec[i] - 4).wrapping_mul(fl2fxconst_dbl(2.0 / 64.0));

            band_energy_ld_data[i] = if band_energy_ld_data[i]
                >= (fl2fxconst_dbl(-1.0) >> 1).wrapping_add(scale_diff >> 1)
            {
                band_energy_ld_data[i].wrapping_sub(scale_diff)
            } else {
                fl2fxconst_dbl(-1.0)
            };
            // find maxNrgLd
            max_nrg_ld = fix_max(max_nrg_ld, band_energy_ld_data[i]);
        }

        if max_nrg_ld <= 0 {
            for i in (0..nb).rev() {
                let scale = fix_min((sfb_max_scale_spec[i] - 4).wrapping_shl(1), DFRACT_BITS - 1);
                band_energy[i] = scale_value(band_energy[i], scale.wrapping_neg());
            }
            0
        } else {
            // scale down NRGs
            while max_nrg_ld > 0 {
                max_nrg_ld -= fl2fxconst_dbl(2.0 / 64.0);
                shift_bits += 1;
            }
            for i in (0..nb).rev() {
                let scale = fix_min(
                    ((sfb_max_scale_spec[i] - 4) + shift_bits).wrapping_shl(1),
                    DFRACT_BITS - 1,
                );
                band_energy_ld_data[i] = band_energy_ld_data[i]
                    .wrapping_sub(shift_bits.wrapping_mul(fl2fxconst_dbl(2.0 / 64.0)));
                band_energy[i] = scale_value(band_energy[i], scale.wrapping_neg());
            }
            shift_bits
        }
    }

    fn fdk_aac_enc_calc_band_energy_optim_short_straight(
        mdct_spectrum: &[FixpDbl],
        sfb_max_scale_spec: &[i32],
        band_offset: &[i32],
        num_bands: i32,
        band_energy: &mut [FixpDbl],
    ) {
        let nb = num_bands.max(0) as usize;
        for i in 0..nb {
            // max sfbWidth = 36 ; 2^6=64 => 6/2 = 3 (spc*spc)
            let leading_bits: i32 = sfb_max_scale_spec[i] - 3;
            let mut tmp: FixpDbl = 0;
            for j in band_offset[i]..band_offset[i + 1] {
                let spec: FixpDbl = scale_value(mdct_spectrum[j as usize], leading_bits);
                tmp = f_pow2_add_div2(tmp, spec);
            }
            band_energy[i] = tmp;
        }

        for i in 0..nb {
            // max sfbWidth = 36 ; 2^6=64 => 6/2 = 3 (spc*spc)
            let mut scale: i32 = (sfb_max_scale_spec[i] - 3).wrapping_mul(2).wrapping_sub(1);
            scale = fix_max(fix_min(scale, DFRACT_BITS - 1), -(DFRACT_BITS - 1));
            band_energy[i] = scale_value_saturate(band_energy[i], -scale);
        }
    }

    fn fdk_aac_enc_calc_band_nrg_ms_opt_straight(
        mdct_spectrum_left: &[FixpDbl],
        mdct_spectrum_right: &[FixpDbl],
        sfb_max_scale_spec_left: &[i32],
        sfb_max_scale_spec_right: &[i32],
        band_offset: &[i32],
        num_bands: i32,
        band_energy_mid: &mut [FixpDbl],
        band_energy_side: &mut [FixpDbl],
        calc_ld_data: i32,
        band_energy_mid_ld_data: &mut [FixpDbl],
        band_energy_side_ld_data: &mut [FixpDbl],
    ) {
        let nb = num_bands.max(0) as usize;

        for i in 0..nb {
            let mut nrg_mid: FixpDbl = 0;
            let mut nrg_side: FixpDbl = 0;
            let mut min_scale =
                fix_min(sfb_max_scale_spec_left[i], sfb_max_scale_spec_right[i]) - 4;
            min_scale = fix_max(0, min_scale);

            if min_scale > 0 {
                for j in band_offset[i] as usize..band_offset[i + 1] as usize {
                    let spec_l: FixpDbl =
                        mdct_spectrum_left[j].wrapping_shl((min_scale - 1) as u32);
                    let spec_r: FixpDbl =
                        mdct_spectrum_right[j].wrapping_shl((min_scale - 1) as u32);
                    let specm = spec_l.wrapping_add(spec_r);
                    let specs = spec_l.wrapping_sub(spec_r);
                    nrg_mid = f_pow2_add_div2(nrg_mid, specm);
                    nrg_side = f_pow2_add_div2(nrg_side, specs);
                }
            } else {
                for j in band_offset[i] as usize..band_offset[i + 1] as usize {
                    let spec_l: FixpDbl = mdct_spectrum_left[j] >> 1;
                    let spec_r: FixpDbl = mdct_spectrum_right[j] >> 1;
                    let specm = spec_l.wrapping_add(spec_r);
                    let specs = spec_l.wrapping_sub(spec_r);
                    nrg_mid = f_pow2_add_div2(nrg_mid, specm);
                    nrg_side = f_pow2_add_div2(nrg_side, specs);
                }
            }
            band_energy_mid[i] = fix_min(nrg_mid, MAXVAL_DBL >> 1) << 1;
            band_energy_side[i] = fix_min(nrg_side, MAXVAL_DBL >> 1) << 1;
        }

        if calc_ld_data != 0 {
            ld_data_vector(&band_energy_mid[..nb], band_energy_mid_ld_data);
            ld_data_vector(&band_energy_side[..nb], band_energy_side_ld_data);
        }

        for i in 0..nb {
            let min_scale = fix_min(sfb_max_scale_spec_left[i], sfb_max_scale_spec_right[i]);
            let mut scale: i32 = fix_max(0, min_scale.wrapping_sub(4).wrapping_mul(2));

            if calc_ld_data != 0 {
                // using the minimal scaling of left and right channel can cause very small
                // energies; check ldNrg before subtract scaling multiplication: fract*INT we don't
                // need fMult
                let minus: i32 = scale.wrapping_mul(fl2fxconst_dbl(1.0 / 64.0));

                if band_energy_mid_ld_data[i] != fl2fxconst_dbl(-1.0) {
                    band_energy_mid_ld_data[i] = band_energy_mid_ld_data[i].wrapping_sub(minus);
                }

                if band_energy_side_ld_data[i] != fl2fxconst_dbl(-1.0) {
                    band_energy_side_ld_data[i] = band_energy_side_ld_data[i].wrapping_sub(minus);
                }
            }
            scale = fix_min(scale, DFRACT_BITS - 1);
            band_energy_mid[i] >>= scale;
            band_energy_side[i] >>= scale;
        }
    }

    #[test]
    fn band_loops_vs_straight() {
        let mut seed = 0x0123_4567_89ab_cdefu64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed >> 32) as u32
        };
        for _ in 0..20_000 {
            let nb = (next() % 52) as usize;
            let mut offsets = vec![0i32; nb + 1];
            for b in 1..=nb {
                offsets[b] = (offsets[b - 1] + (next() % 25) as i32).min(1024);
            }
            if next() % 8 == 0 && nb > 1 {
                // an empty or inverted band
                offsets[1] = offsets[0];
            }
            let kind = next() % 4;
            let mut spec = || -> Vec<i32> {
                (0..1024)
                    .map(|_| match kind {
                        0 => next() as i32,
                        1 => (next() as i32) >> (next() % 32),
                        2 => [0, 1, -1, i32::MIN, i32::MAX][(next() % 5) as usize],
                        _ => (next() as i32) >> 12,
                    })
                    .collect()
            };
            let (l, r) = (spec(), spec());
            let scale = |next: &mut dyn FnMut() -> u32| -> Vec<i32> {
                (0..nb.max(1)).map(|_| (next() % 31) as i32).collect()
            };
            let (sl, sr) = (scale(&mut next), scale(&mut next));
            let n = nb as i32;

            let (mut a, mut b) = (vec![0; nb.max(1)], vec![0; nb.max(1)]);
            fdk_aac_enc_calc_sfb_max_scale_spec(&l, &offsets, &mut a, n);
            fdk_aac_enc_calc_sfb_max_scale_spec_straight(&l, &offsets, &mut b, n);
            assert_eq!(a, b);

            let (mut ea, mut la, mut eb, mut lb) =
                (vec![0; 64], vec![0; 64], vec![0; 64], vec![0; 64]);
            if nb > 0 {
                let ms = (next() % 30) as i32 - 4;
                let ra =
                    fdk_aac_enc_check_band_energy_optim(&l, &sl, &offsets, n, &mut ea, &mut la, ms);
                let rb = fdk_aac_enc_check_band_energy_optim_straight(
                    &l, &sl, &offsets, n, &mut eb, &mut lb, ms,
                );
                assert_eq!((ra, &ea, &la), (rb, &eb, &lb));
            }

            let ra =
                fdk_aac_enc_calc_band_energy_optim_long(&l, &sl, &offsets, n, &mut ea, &mut la);
            let rb = fdk_aac_enc_calc_band_energy_optim_long_straight(
                &l, &sl, &offsets, n, &mut eb, &mut lb,
            );
            assert_eq!((ra, &ea, &la), (rb, &eb, &lb));

            fdk_aac_enc_calc_band_energy_optim_short(&l, &sl, &offsets, n, &mut ea);
            fdk_aac_enc_calc_band_energy_optim_short_straight(&l, &sl, &offsets, n, &mut eb);
            assert_eq!(ea, eb);

            let calc_ld = (next() % 2) as i32;
            let (mut ma, mut sa, mut mla, mut sla) =
                (vec![1; 64], vec![2; 64], vec![3; 64], vec![4; 64]);
            let (mut mb, mut sb, mut mlb, mut slb) =
                (ma.clone(), sa.clone(), mla.clone(), sla.clone());
            fdk_aac_enc_calc_band_nrg_ms_opt(
                &l, &r, &sl, &sr, &offsets, n, &mut ma, &mut sa, calc_ld, &mut mla, &mut sla,
            );
            fdk_aac_enc_calc_band_nrg_ms_opt_straight(
                &l, &r, &sl, &sr, &offsets, n, &mut mb, &mut sb, calc_ld, &mut mlb, &mut slb,
            );
            assert_eq!((&ma, &sa, &mla, &sla), (&mb, &sb, &mlb, &slb));
        }
    }
}
