// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACenc/src/noisedet.h, libAACenc/src/noisedet.cpp.

//! Port of `noisedet.h` and `noisedet.cpp`: routines for noise detection.

use crate::pnsparam::{NoiseParams, USE_POWER_DISTRIBUTION, USE_PSYCH_TONALITY};
use fdk_aac_rs_fdk::common_fix::*;

/// `FDKaacEnc_fuzzyIsSmaller(testVal, refVal, loLim, hiLim)`: fuzzy value calculation for
/// "testVal is smaller than refVal".
pub fn fdk_aac_enc_fuzzy_is_smaller(
    test_val: FixpDbl,
    ref_val: FixpDbl,
    lo_lim: FixpDbl,
    hi_lim: FixpDbl,
) -> FixpSgl {
    if ref_val <= fl2fxconst_dbl(0.0) {
        fl2fxconst_sgl(0.0)
    } else if test_val >= f_mult((hi_lim >> 1) + (lo_lim >> 1), ref_val) {
        fl2fxconst_sgl(0.0)
    } else {
        MAXVAL_SGL as FixpSgl
    }
}

thread_local! {
    static TONALITY_READS: core::cell::Cell<u64> = const { core::cell::Cell::new(0) };
}

/// The reads of `sfbtonality[]` by [`fdk_aac_enc_noise_detect`] on this thread (test support for
/// [`psy_main_lfe_flag_pairs`](crate::psy_main::psy_main_lfe_flag_pairs)).
pub(crate) fn noise_detect_tonality_reads() -> u64 {
    TONALITY_READS.with(|c| c.get())
}

/// `FDKaacEnc_noiseDetect(mdctSpectrum, sfbMaxScaleSpec, sfbActive, sfbOffset,
/// noiseFuzzyMeasure, np, sfbtonality)`: detect tonal sfb's; two tests.
///
/// - Powerdistribution: sfb splittet in four regions, compare the energy in all sections.
/// - PsychTonality: compare tonality from chaosmeasure with reftonality.
///
/// Output: `noiseFuzzyMeasure[0..sfbActive]`.
pub fn fdk_aac_enc_noise_detect(
    mdct_spectrum: &[FixpDbl],
    sfb_max_scale_spec: &[i32],
    sfb_active: i32,
    sfb_offset: &[i32],
    noise_fuzzy_measure: &mut [FixpSgl],
    np: &NoiseParams,
    sfbtonality: &[FixpSgl],
) {
    let mut tonality_reads: u64 = 0;
    // ***** Start detection phase *****
    // Start noise detection for each band based on a number of checks
    for sfb in 0..sfb_active.max(0) as usize {
        let mut fuzzy_total: FixpSgl = MAXVAL_SGL as FixpSgl;
        let sfb_width = sfb_offset[sfb + 1].wrapping_sub(sfb_offset[sfb]);

        // Reset output for lower bands or too small bands
        if (sfb as i32) < np.start_sfb as i32 || sfb_width < np.min_sfb_width {
            noise_fuzzy_measure[sfb] = fl2fxconst_sgl(0.0);
            continue;
        }

        if (np.detection_algorithm_flags & USE_POWER_DISTRIBUTION) != 0
            && (fuzzy_total > fl2fxconst_sgl(0.5))
        {
            // max sfbWidth = 96/4 ; 2^5=32 => 5/2 = 3 (spc*spc)
            let mut leading_bits = fix_max(0, sfb_max_scale_spec[sfb].wrapping_sub(3));

            // check power distribution in four regions
            let mut fhelp1 = fl2fxconst_dbl(0.0);
            let mut fhelp2 = fl2fxconst_dbl(0.0);
            let mut fhelp3 = fl2fxconst_dbl(0.0);
            let mut fhelp4 = fl2fxconst_dbl(0.0);
            let k = (sfb_width >> 2) as isize; // Width of a quarter band

            // The shifts by leadingBits compile to `lsl`/`shl` with a register amount on both
            // reference builds (amount modulo 32).
            let sh = |x: FixpDbl, n: i32| x.wrapping_shl(n as u32);
            let start = sfb_offset[sfb] as isize;
            let mut i = start;
            while i < start + k {
                let at = |j: isize| mdct_spectrum[j as usize];
                fhelp1 = f_pow2_add_div2(fhelp1, sh(at(i), leading_bits));
                fhelp2 = f_pow2_add_div2(fhelp2, sh(at(i + k), leading_bits));
                fhelp3 = f_pow2_add_div2(fhelp3, sh(at(i + 2 * k), leading_bits));
                fhelp4 = f_pow2_add_div2(fhelp4, sh(at(i + 3 * k), leading_bits));
                i += 1;
            }

            // get max into fhelp:
            let mut max_val = fix_max(fhelp1, fhelp2);
            max_val = fix_max(max_val, fhelp3);
            max_val = fix_max(max_val, fhelp4);

            // get min into fhelp1:
            let mut min_val = fix_min(fhelp1, fhelp2);
            min_val = fix_min(min_val, fhelp3);
            min_val = fix_min(min_val, fhelp4);

            // Normalize min and max Val
            leading_bits = count_leading_bits(max_val);
            let mut test_val = sh(max_val, leading_bits);
            let ref_val = sh(min_val, leading_bits);

            // calculate fuzzy value for power distribution
            test_val = f_mult_div2(test_val, np.pow_dist_psd_curve[sfb]);

            let fuzzy = fdk_aac_enc_fuzzy_is_smaller(
                test_val,              // 1/2 * maxValue * PSDcurve
                ref_val,               // 1   * minValue
                fl2fxconst_dbl(0.495), // 1/2 * loLim  (0.99f/2)
                fl2fxconst_dbl(0.505), // 1/2 * hiLim  (1.01f/2)
            );

            fuzzy_total = fix_min(fuzzy_total, fuzzy);
        }

        if (np.detection_algorithm_flags & USE_PSYCH_TONALITY) != 0
            && (fuzzy_total > fl2fxconst_sgl(0.5))
        {
            // Detection with tonality-value of psych. acoustic (here: 1 is tonal!)

            tonality_reads += 1;
            let test_val = fx_sgl2fx_dbl(sfbtonality[sfb]) >> 1; // 1/2 * sfbTonality
            let ref_val = np.ref_tonality;

            let fuzzy = fdk_aac_enc_fuzzy_is_smaller(
                test_val,
                ref_val,
                fl2fxconst_dbl(0.45f32 as f64), // 1/2 * loLim  (0.9f/2)
                fl2fxconst_dbl(0.55f32 as f64), // 1/2 * hiLim  (1.1f/2)
            );

            fuzzy_total = fix_min(fuzzy_total, fuzzy);
        }

        // Output of final result
        noise_fuzzy_measure[sfb] = fuzzy_total;
    }
    if tonality_reads != 0 {
        TONALITY_READS.with(|c| c.set(c.get().wrapping_add(tonality_reads)));
    }
}
