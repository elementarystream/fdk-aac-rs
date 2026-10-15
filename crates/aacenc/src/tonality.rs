// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACenc/src/tonality.h, libAACenc/src/tonality.cpp.

//! Port of `tonality.h`: convert chaos measure to the tonality index.
//!
//! The `aarch64` and `x86` translation units of `tonality.cpp` are identical; the variants
//! differ only in the primitives.
//!
//! The chaos measure scratch buffer (`C_ALLOC_SCRATCH_START`, not initialized by the reference)
//! is a zeroed local buffer (see [`chaosmeasure`](crate::chaosmeasure)).

use crate::chaosmeasure::fdk_aac_enc_calculate_chaos_measure;
use fdk_aac_rs_fdk::common_fix::*;

/// `normlog`: `FL2FXCONST_DBL(-0.4342944819f * FDKlog(2.0)/FDKlog(2.7182818))`.
/// `scripts/dump_table.py libAACenc/src/tonality.cpp '&normlog' LONG --count 1 --hex`
pub const NORMLOG: FixpDbl = 0xD977D949u32 as i32;

/// `FDKaacEnc_CalculateFullTonality()`: tonality of the `sfb_cnt` scalefactor bands of
/// `spectrum` (only with `use_pns`; `sfb_tonality` is not written otherwise).
pub fn fdk_aac_enc_calculate_full_tonality(
    spectrum: &[FixpDbl],
    sfb_max_scale_spec: &[i32],
    sfb_energy_ld64: &[FixpDbl],
    sfb_tonality: &mut [FixpSgl],
    sfb_cnt: i32,
    sfb_offset: &[i32],
    use_pns: i32,
) {
    let number_of_lines = sfb_offset[sfb_cnt as usize];

    if use_pns != 0 {
        let mut chaos_measure_per_line = [0 as FixpDbl; 1024];

        // calculate chaos measure
        fdk_aac_enc_calculate_chaos_measure(spectrum, number_of_lines, &mut chaos_measure_per_line);

        // smooth ChaosMeasure
        let cm = &mut chaos_measure_per_line;
        let mut left: FixpDbl = cm[0];
        let mut right: FixpDbl;
        let mut j: i32 = 1;
        while j < number_of_lines - 1 {
            let ju = j as usize;
            right = cm[ju];
            right = right.wrapping_sub(right >> 2);
            left = right.wrapping_add(left >> 2);
            cm[ju] = left; // 0.25 left + 0.75 right

            right = cm[ju + 1];
            right = right.wrapping_sub(right >> 2);
            left = right.wrapping_add(left >> 2);
            cm[ju + 1] = left;
            j += 2;
        }
        if j == number_of_lines - 1 {
            let ju = j as usize;
            right = cm[ju];
            right = right.wrapping_sub(right >> 2);
            left = right.wrapping_add(left >> 2);
            cm[ju] = left;
        }

        fdk_aac_enc_calc_sfb_tonality(
            spectrum,
            sfb_max_scale_spec,
            cm,
            sfb_tonality,
            sfb_cnt,
            sfb_offset,
            sfb_energy_ld64,
        );
    }
}

/// `FDKaacEnc_CalcSfbTonality()` (static in C, `CalculateTonalityIndex`): computes tonality
/// values out of unpredictability values, limits range and computes log().
pub fn fdk_aac_enc_calc_sfb_tonality(
    spectrum: &[FixpDbl],
    sfb_max_scale_spec: &[i32],
    chaos_measure: &[FixpDbl],
    sfb_tonality: &mut [FixpSgl],
    sfb_cnt: i32,
    sfb_offset: &[i32],
    sfb_energy_ld64: &[FixpDbl],
) {
    // `spectrum` and `chaosMeasure` are walked with one pointer each across the bands.
    let mut k = 0usize;
    for i in 0..sfb_cnt.max(0) as usize {
        // max sfbWidth = 96 ; 2^7=128 => 7/2 = 4 (spc*spc)
        let shift_bits: i32 = fix_max(0, sfb_max_scale_spec[i] - 4);

        let mut chaos_measure_sfb: FixpDbl = 0;

        // calc chaosMeasurePerSfb
        let mut j = (sfb_offset[i + 1] - sfb_offset[i]) - 1;
        while j >= 0 {
            // shiftBits is at most 26 (sfbMaxScaleSpec <= 30); modulo 32 as the shift with a
            // register amount of both reference builds.
            let tmp: FixpDbl = spectrum[k].wrapping_shl(shift_bits as u32);
            let line_nrg: FixpDbl = f_mult_div2(tmp, tmp);
            chaos_measure_sfb = f_mult_add_div2(chaos_measure_sfb, line_nrg, chaos_measure[k]);
            k += 1;
            j -= 1;
        }

        // calc tonalityPerSfb
        if chaos_measure_sfb != 0 {
            // add ld(convtone)/64 and 2/64 bec.fMultDiv2
            let mut chaos_measure_sfb_ld64: FixpDbl =
                calc_ld_data(chaos_measure_sfb).wrapping_sub(sfb_energy_ld64[i]);
            chaos_measure_sfb_ld64 = chaos_measure_sfb_ld64.wrapping_add(
                fl2fxconst_dbl(3.0 / 64.0)
                    .wrapping_sub(shift_bits.wrapping_shl((DFRACT_BITS - 6) as u32)),
            );

            if chaos_measure_sfb_ld64 > fl2fxconst_dbl(-0.0519051) {
                // > ld(0.05)+ld(2)
                if chaos_measure_sfb_ld64 <= 0 {
                    sfb_tonality[i] =
                        fx_dbl2fx_sgl(f_mult_div2(chaos_measure_sfb_ld64, NORMLOG) << 7);
                } else {
                    sfb_tonality[i] = 0;
                }
            } else {
                sfb_tonality[i] = MAXVAL_SGL as FixpSgl;
            }
        } else {
            sfb_tonality[i] = MAXVAL_SGL as FixpSgl;
        }
    }
}
