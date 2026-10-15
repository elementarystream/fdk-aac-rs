// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACenc/src/chaosmeasure.h, libAACenc/src/chaosmeasure.cpp.

//! Port of `chaosmeasure.h`: chaos measure calculation.
//!
//! The `aarch64` and `x86` translation units of `chaosmeasure.cpp` are identical; the variants
//! differ in the primitives, in particular in `schur_div` (64 bit division on x86).
//!
//! Where the reference has no defined result, the port reads the caller's buffer as it is and
//! drops writes outside `chaos_measure`.

use fdk_aac_rs_fdk::common_fix::*;

/// `|x|` as the "peak filter" computes it: `x ^ (x >> 31)` (one's complement of negative
/// values).
#[inline(always)]
fn abs1(x: FixpDbl) -> FixpDbl {
    x ^ (x >> (DFRACT_BITS - 1))
}

/// `FDKaacEnc_FDKaacEnc_CalculateChaosMeasurePeakFast()` (static in C): Eberlein method of
/// chaos measure calculation by high-pass filtering amplitude spectrum. A special case of
/// FDKaacEnc_CalculateChaosMeasureTonalGeneric -- highly optimized.
///
/// Reads `max(4, number_of_lines)` spectral lines.
pub fn fdk_aac_enc_fdk_aac_enc_calculate_chaos_measure_peak_fast(
    pa_mdct_data_nm0: &[FixpDbl],
    number_of_lines: i32,
    chaos_measure: &mut [FixpDbl],
) {
    // calculate chaos measure by "peak filter"
    // make even and odd pass through data

    // left, center tap of filter, even numbered
    let mut left_0_div2: FixpDbl = abs1(pa_mdct_data_nm0[0]) >> 1;
    // left, center tap of filter, odd numbered
    let mut left_1_div2: FixpDbl = abs1(pa_mdct_data_nm0[1]) >> 1;
    let mut center_0: FixpDbl = abs1(pa_mdct_data_nm0[2]);
    let mut center_1: FixpDbl = abs1(pa_mdct_data_nm0[3]);

    let mut j: i32 = 2;
    while j < number_of_lines - 2 {
        let ju = j as usize;
        let right_0: FixpDbl = abs1(pa_mdct_data_nm0[ju + 2]);
        let mut tmp_0: FixpDbl = left_0_div2.wrapping_add(right_0 >> 1);
        let right_1: FixpDbl = abs1(pa_mdct_data_nm0[ju + 3]);
        let mut tmp_1: FixpDbl = left_1_div2.wrapping_add(right_1 >> 1);

        if tmp_0 < center_0 {
            let leading_bits: i32 = cnt_leading_zeros(center_0) - 1;
            tmp_0 = schur_div(tmp_0 << leading_bits, center_0 << leading_bits, 8);
            tmp_0 = f_mult(tmp_0, tmp_0);
        } else {
            tmp_0 = MAXVAL_DBL;
        }
        chaos_measure[ju] = tmp_0;
        left_0_div2 = center_0 >> 1;
        center_0 = right_0;

        if tmp_1 < center_1 {
            let leading_bits: i32 = cnt_leading_zeros(center_1) - 1;
            tmp_1 = schur_div(tmp_1 << leading_bits, center_1 << leading_bits, 8);
            tmp_1 = f_mult(tmp_1, tmp_1);
        } else {
            tmp_1 = MAXVAL_DBL;
        }

        left_1_div2 = center_1 >> 1;
        center_1 = right_1;
        chaos_measure[ju + 1] = tmp_1;
        j += 2;
    }

    // provide chaos measure for first few lines
    chaos_measure[0] = chaos_measure[2];
    chaos_measure[1] = chaos_measure[2];

    // provide chaos measure for last few lines
    for i in (number_of_lines - 3)..number_of_lines {
        if let Some(c) = usize::try_from(i).ok().and_then(|i| chaos_measure.get_mut(i)) {
            *c = fl2fxconst_dbl(0.5);
        }
    }
}

/// `FDKaacEnc_CalculateChaosMeasure()`: calculates a chaosmeasure for every line, different
/// methods are available. 0 means tonal, 1 means noiselike.
pub fn fdk_aac_enc_calculate_chaos_measure(
    pa_mdct_data_nm0: &[FixpDbl],
    number_of_lines: i32,
    chaos_measure: &mut [FixpDbl],
) {
    fdk_aac_enc_fdk_aac_enc_calculate_chaos_measure_peak_fast(
        pa_mdct_data_nm0,
        number_of_lines,
        chaos_measure,
    );
}
