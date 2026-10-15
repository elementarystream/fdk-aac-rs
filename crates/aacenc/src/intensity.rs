// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACenc/src/intensity.h, libAACenc/src/intensity.cpp.

//! Port of `intensity.h` and `intensity.cpp`: intensity stereo processing.
//!
//! The array parameters are the slices the caller (`FDKaacEnc_psyMain()`) passes: the `.Long`
//! views of the `PSY_DATA` unions (`sfbEnergy`, `sfbThreshold`, `sfbSpreadEnergy`), the arrays of
//! `QC_OUT_CHANNEL` that `PSY_OUT_CHANNEL` points to (`mdctSpectrum`, `sfbEnergyLdData`,
//! `sfbThresholdLdData`), `TOOLSINFO::msMask`, `PSY_OUT_CHANNEL::isBook`/`isScale` and the
//! `PNS_DATA` of both channels. The indexes `sfb + sfboffs` stay below `sfbCnt` (at most
//! `MAX_GROUPED_SFB`) and `sfbOffset[]` below the frame length for the consistent parameters of
//! the encoder (`maxSfbPerGroup <= sfbPerGroup`); the slices must cover them.
//! `FDKaacEnc_psyMain()` refuses the inputs without a defined reference result before (see
//! [`psy_main_mixed_block_pairs`](crate::psy_main::psy_main_mixed_block_pairs)).

use crate::aacenc_pns::PnsData;
use crate::bit_cnt::CODE_BOOK_IS_IN_PHASE_NO;
use crate::interface::MS_SOME;
use crate::psy_const::{MAX_GROUPED_SFB, MAX_NO_OF_GROUPS};
use fdk_aac_rs_fdk::common_fix::*;
use fdk_aac_rs_fdk::fixpoint_math::{f_div_norm, get_inv_int, sqrt_fixp, LD_DATA_SHIFT};
use fdk_aac_rs_fdk::scale::scale_value;

/// only set an IS seed it left/right channel correlation is above IS_CORR_THRESH
const IS_CORR_THRESH: FixpDbl = fl2fxconst_dbl(0.95f32 as f64);

/// when expanding the IS region to more SFBs only accept an error that is not more than
/// IS_TOTAL_ERROR_THRESH overall and not more than IS_LOCAL_ERROR_THRESH for the current SFB
const IS_TOTAL_ERROR_THRESH: FixpDbl = fl2fxconst_dbl(0.04f32 as f64);
const IS_LOCAL_ERROR_THRESH: FixpDbl = fl2fxconst_dbl(0.01f32 as f64);

/// the maximum allowed change of the intensity direction (unit: IS scale) - scaled with factor
/// 0.25 -
const IS_DIRECTION_DEVIATION_THRESH_SF: i32 = 2;
const IS_DIRECTION_DEVIATION_THRESH: FixpDbl =
    fl2fxconst_dbl((2.0f32 / (1 << IS_DIRECTION_DEVIATION_THRESH_SF) as f32) as f64);

/// IS regions need to have a minimal percentage of the overall loudness, e.g. 0.06 == 6%
const IS_REGION_MIN_LOUDNESS: FixpDbl = fl2fxconst_dbl(0.1f32 as f64);

/// only perform IS if IS_MIN_SFBS neighboring SFBs can be processed
const IS_MIN_SFBS: i32 = 6;

/// only do IS if IS_LEFT_RIGHT_RATIO_THRESH < sfbEnergyLeft[sfb]/sfbEnergyRight[sfb] < 1 /
/// IS_LEFT_RIGHT_RATIO_THRESH -> no IS if the panning angle is not far from the middle, MS will
/// do. This is equivalent to a scale of +/-1.02914634566
const IS_LEFT_RIGHT_RATIO_THRESH: FixpDbl = fl2fxconst_dbl(0.7f32 as f64);

/// scalefactor of realScale
const REAL_SCALE_SF: i32 = 1;

/// scalefactor overallLoudness
const OVERALL_LOUDNESS_SF: i32 = 6;

/// scalefactor for sum over max samples per goup
const MAX_SFB_PER_GROUP_SF: i32 = 6;

/// scalefactor for sum of mdct spectrum
const MDCT_SPEC_SF: i32 = 6;

/// `INTENSITY_PARAMETERS` (file-local in C)
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct IntensityParameters {
    /// Only set an IS seed it left/right channel correlation is above corr_thresh
    pub corr_thresh: FixpDbl,
    /// When expanding the IS region to more SFBs only accept an error that is not more than
    /// 'total_error_thresh' overall.
    pub total_error_thresh: FixpDbl,
    /// When expanding the IS region to more SFBs only accept an error that is not more than
    /// 'local_error_thresh' for the current SFB.
    pub local_error_thresh: FixpDbl,
    /// The maximum allowed change of the intensity direction (unit: IS scale)
    pub direction_deviation_thresh: FixpDbl,
    /// IS regions need to have a minimal percentage of the overall loudness, e.g. 0.06 == 6%
    pub is_region_min_loudness: FixpDbl,
    /// Only perform IS if 'min_is_sfbs' neighboring SFBs can be processed
    pub min_is_sfbs: i32,
    /// No IS if the panning angle is not far from the middle, MS will do
    pub left_right_ratio_threshold: FixpDbl,
}

/// `calcSfbMaxScale(const FIXP_DBL *mdctSpectrum, const INT l1, const INT l2)` (file-static):
/// calc max value in scalefactor band; returns the scalefactor.
///
/// `fixp_abs(MINVAL_DBL)` is `MINVAL_DBL` (negative), which never raises the maximum; the
/// vectorized reference code (per-lane `smax` from 0) gives the same.
pub fn calc_sfb_max_scale(mdct_spectrum: &[FixpDbl], l1: i32, l2: i32) -> i32 {
    // (over the band's slice; the maximum does not depend on the order)
    let max_spc: FixpDbl =
        band(mdct_spectrum, l1, l2).iter().fold(0, |m, &x| fix_max(m, fixp_abs(x)));
    if max_spc == 0 {
        DFRACT_BITS - 2
    } else {
        cnt_leading_zeros(max_spc) - 1
    }
}

/// The lines `lo..hi` of `x` (none for `hi <= lo`, where the C loops do not run).
#[inline(always)]
fn band<T>(x: &[T], lo: i32, hi: i32) -> &[T] {
    if lo < hi {
        &x[lo as usize..hi as usize]
    } else {
        &[]
    }
}

/// `FDKaacEnc_initIsParams(INTENSITY_PARAMETERS *isParams)` (file-static): initialization of
/// intensity parameters.
pub fn fdk_aac_enc_init_is_params(is_params: &mut IntensityParameters) {
    is_params.corr_thresh = IS_CORR_THRESH;
    is_params.total_error_thresh = IS_TOTAL_ERROR_THRESH;
    is_params.local_error_thresh = IS_LOCAL_ERROR_THRESH;
    is_params.direction_deviation_thresh = IS_DIRECTION_DEVIATION_THRESH;
    is_params.is_region_min_loudness = IS_REGION_MIN_LOUDNESS;
    is_params.min_is_sfbs = IS_MIN_SFBS;
    is_params.left_right_ratio_threshold = IS_LEFT_RIGHT_RATIO_THRESH;
}

fdk_aac_rs_fdk::avx2_clone! {
/// `FDKaacEnc_prepareIntensityDecision(...)` (file-static): prepares intensity decision.
///
/// Outputs: `hrr_err` (scale: none), `is_mask` (scale: none), `real_scale` (scale:
/// `LD_DATA_SHIFT + REAL_SCALE_SF`), `norm_sfb_loudness` (scale: none). `real_scale` and
/// `norm_sfb_loudness` are cleared for `MAX_GROUPED_SFB` entries.
///
/// On x86 the function is compiled a second time with AVX2 (`avx2_clone!`), so that
/// the band sums use 256-bit `vpmuldq` products (the same Rust code).
pub fn fdk_aac_enc_prepare_intensity_decision(
    sfb_energy_left: &[FixpDbl],
    sfb_energy_right: &[FixpDbl],
    sfb_energy_ld_data_left: &[FixpDbl],
    sfb_energy_ld_data_right: &[FixpDbl],
    mdct_spectrum_left: &[FixpDbl],
    mdct_spectrum_right: &[FixpDbl],
    is_params: &IntensityParameters,
    hrr_err: &mut [FixpDbl],
    is_mask: &mut [i32],
    real_scale: &mut [FixpDbl],
    norm_sfb_loudness: &mut [FixpDbl],
    sfb_cnt: i32,
    sfb_per_group: i32,
    max_sfb_per_group: i32,
    sfb_offset: &[i32],
) {
    // temporary variables to compute loudness
    let mut overall_loudness = [0 as FixpDbl; MAX_NO_OF_GROUPS];

    // temporary variables to compute correlation
    let mut channel_corr = [0 as FixpDbl; MAX_GROUPED_SFB];

    norm_sfb_loudness[..MAX_GROUPED_SFB].fill(0);
    real_scale[..MAX_GROUPED_SFB].fill(0);

    let mut grp_counter = 0usize;
    let mut sfboffs = 0;
    while sfboffs < sfb_cnt {
        overall_loudness[grp_counter] = 0;
        for sfb in 0..max_sfb_per_group {
            let k = (sfb + sfboffs) as usize;
            let is_value = sfb_energy_ld_data_left[k].wrapping_sub(sfb_energy_ld_data_right[k]);

            // delimitate intensity scale value to representable range
            real_scale[k] = fix_min(
                fl2fxconst_dbl((60.0f32 / (1 << (REAL_SCALE_SF + LD_DATA_SHIFT)) as f32) as f64),
                fix_max(
                    fl2fxconst_dbl(
                        (-60.0f32 / (1 << (REAL_SCALE_SF + LD_DATA_SHIFT)) as f32) as f64,
                    ),
                    is_value,
                ),
            );

            let s_l = fix_max(0, cnt_leading_zeros(sfb_energy_left[k]) - 1);
            let s_r = fix_max(0, cnt_leading_zeros(sfb_energy_right[k]) - 1);
            let s = (fix_min(s_l, s_r) >> 2) << 2;
            norm_sfb_loudness[k] = sqrt_fixp(sqrt_fixp(
                ((sfb_energy_left[k] << s) >> 1) + ((sfb_energy_right[k] << s) >> 1),
            )) >> (s >> 2);

            overall_loudness[grp_counter] = overall_loudness[grp_counter]
                .wrapping_add(norm_sfb_loudness[k] >> OVERALL_LOUDNESS_SF);
            // don't do intensity if
            // - panning angle is too close to the middle or
            // - one channel is non-existent or
            // - if it is dual mono
            if (sfb_energy_left[k]
                >= f_mult(is_params.left_right_ratio_threshold, sfb_energy_right[k]))
                && (f_mult(is_params.left_right_ratio_threshold, sfb_energy_left[k])
                    <= sfb_energy_right[k])
            {
                // this will prevent post processing from considering this SFB for merging
                hrr_err[k] = fl2fxconst_dbl(1.0 / 8.0);
            }
        }
        sfboffs += sfb_per_group;
        grp_counter += 1;
    }

    let mut grp_counter = 0usize;
    let mut sfboffs = 0;
    while sfboffs < sfb_cnt {
        let mut inv_overall_loudness_sf: i32;
        let inv_overall_loudness: FixpDbl;

        if overall_loudness[grp_counter] == 0 {
            inv_overall_loudness = 0;
            inv_overall_loudness_sf = 0;
        } else {
            inv_overall_loudness_sf = 0;
            inv_overall_loudness =
                f_div_norm(MAXVAL_DBL, overall_loudness[grp_counter], &mut inv_overall_loudness_sf);
            // +1: compensate fMultDiv2() in subsequent loop
            inv_overall_loudness_sf = inv_overall_loudness_sf - OVERALL_LOUDNESS_SF + 1;
        }
        inv_overall_loudness_sf =
            fix_min(fix_max(inv_overall_loudness_sf, -(DFRACT_BITS - 1)), DFRACT_BITS - 1);

        for sfb in 0..max_sfb_per_group {
            let k = (sfb + sfboffs) as usize;

            let tmp = f_mult_div2(
                (norm_sfb_loudness[k] >> OVERALL_LOUDNESS_SF) << OVERALL_LOUDNESS_SF,
                inv_overall_loudness,
            );

            norm_sfb_loudness[k] = scale_value(tmp, inv_overall_loudness_sf);

            channel_corr[k] = 0;

            // max width of scalefactorband is 96; width's are always even
            // inv_n is scaled with factor 2 to compensate fMultDiv2() in subsequent loops
            let inv_n = get_inv_int((sfb_offset[k + 1] - sfb_offset[k]) >> 1);

            if inv_n > 0 {
                // correlation := Pearson's product-moment coefficient
                // compute correlation between channels and check if it is over threshold
                let mut ml: FixpDbl = 0;
                let mut mr: FixpDbl = 0;
                let mut prod_lr: FixpDbl = 0;
                let mut square_l: FixpDbl = 0;
                let mut square_r: FixpDbl = 0;

                let mut s_l =
                    calc_sfb_max_scale(mdct_spectrum_left, sfb_offset[k], sfb_offset[k + 1]);
                let mut s_r =
                    calc_sfb_max_scale(mdct_spectrum_right, sfb_offset[k], sfb_offset[k + 1]);
                let mut s = fix_min(s_l, s_r);

                // (over the band's slices; the wrapping sums do not depend on the order)
                let left = band(mdct_spectrum_left, sfb_offset[k], sfb_offset[k + 1]);
                let right = band(mdct_spectrum_right, sfb_offset[k], sfb_offset[k + 1]);
                for (&l, &r) in left.iter().zip(right) {
                    // scaled with mdctScale - s + inv_n
                    ml = ml.wrapping_add(f_mult_div2(l << s, inv_n));
                    // scaled with mdctScale - s + inv_n
                    mr = mr.wrapping_add(f_mult_div2(r << s, inv_n));
                }
                ml = f_mult_div2(ml, inv_n); // scaled with mdctScale - s + inv_n
                mr = f_mult_div2(mr, inv_n); // scaled with mdctScale - s + inv_n

                for (&l, &r) in left.iter().zip(right) {
                    // scaled with mdctScale - s + inv_n
                    let tmp_l = f_mult_div2(l << s, inv_n).wrapping_sub(ml);
                    // scaled with mdctScale - s + inv_n
                    let tmp_r = f_mult_div2(r << s, inv_n).wrapping_sub(mr);

                    // scaled with 2*(mdctScale - s + inv_n) + 1
                    prod_lr = prod_lr.wrapping_add(f_mult_div2(tmp_l, tmp_r));
                    // scaled with 2*(mdctScale - s + inv_n) + 1
                    square_l = square_l.wrapping_add(f_pow2_div2(tmp_l));
                    // scaled with 2*(mdctScale - s + inv_n) + 1
                    square_r = square_r.wrapping_add(f_pow2_div2(tmp_r));
                }
                prod_lr <<= 1; // scaled with 2*(mdctScale - s + inv_n)
                square_l <<= 1; // scaled with 2*(mdctScale - s + inv_n)
                square_r <<= 1; // scaled with 2*(mdctScale - s + inv_n)

                if square_l > 0 && square_r > 0 {
                    let mut channel_corr_sf = 0;

                    // local scaling of square_l and square_r is compensated after sqrt calculation
                    s_l = fix_max(0, cnt_leading_zeros(square_l) - 1);
                    s_r = fix_max(0, cnt_leading_zeros(square_r) - 1);
                    s = ((s_l + s_r) >> 1) << 1;
                    s_l = fix_min(s_l, s);
                    s_r = s - s_l;
                    let mut tmp = f_mult(square_l << s_l, square_r << s_r);
                    tmp = sqrt_fixp(tmp);

                    fdk_aac_rs_fdk::fdk_assert!(tmp > 0);

                    // numerator and denominator have the same scaling
                    if prod_lr < 0 {
                        channel_corr[k] =
                            f_div_norm(prod_lr.wrapping_neg(), tmp, &mut channel_corr_sf)
                                .wrapping_neg();
                    } else {
                        channel_corr[k] = f_div_norm(prod_lr, tmp, &mut channel_corr_sf);
                    }
                    channel_corr_sf = fix_min(
                        fix_max(channel_corr_sf + ((s_l + s_r) >> 1), -(DFRACT_BITS - 1)),
                        DFRACT_BITS - 1,
                    );

                    if channel_corr_sf < 0 {
                        channel_corr[k] >>= -channel_corr_sf;
                    } else {
                        // avoid overflows due to limited computational accuracy
                        if f_abs(channel_corr[k]) > (MAXVAL_DBL >> channel_corr_sf) {
                            if channel_corr[k] < 0 {
                                channel_corr[k] = -MAXVAL_DBL;
                            } else {
                                channel_corr[k] = MAXVAL_DBL;
                            }
                        } else {
                            channel_corr[k] <<= channel_corr_sf;
                        }
                    }
                }
            }

            // for post processing: hrrErr is the error in terms of (too little) correlation
            // weighted with the loudness of the SFB; SFBs with small hrrErr can be merged
            if hrr_err[k] == fl2fxconst_dbl(1.0 / 8.0) {
                continue;
            }

            hrr_err[k] = f_mult_div2(
                fl2fxconst_dbl(0.25f32 as f64).wrapping_sub(channel_corr[k] >> 2),
                norm_sfb_loudness[k],
            );

            // set IS mask/vector to 1, if correlation is high enough
            if f_abs(channel_corr[k]) >= is_params.corr_thresh {
                is_mask[k] = 1;
            }
        }
        sfboffs += sfb_per_group;
        grp_counter += 1;
    }
}
}

/// `FDKaacEnc_finalizeIntensityDecision(...)` (file-static): finalizes intensity decision.
///
/// Inputs: `is_params` (scale: none), `hrr_err` (scale: none), `real_is_scale` (scale:
/// `LD_DATA_SHIFT + REAL_SCALE_SF`), `norm_sfb_loudness` (scale: none). Output: `is_mask`.
#[allow(clippy::too_many_arguments)]
pub fn fdk_aac_enc_finalize_intensity_decision(
    hrr_err: &[FixpDbl],
    is_mask: &mut [i32],
    real_is_scale: &[FixpDbl],
    norm_sfb_loudness: &[FixpDbl],
    is_params: &IntensityParameters,
    sfb_cnt: i32,
    sfb_per_group: i32,
    max_sfb_per_group: i32,
) {
    let mut is_scale_last: FixpDbl = 0;
    let mut is_start_value_found = 0;

    let mut sfboffs = 0;
    while sfboffs < sfb_cnt {
        let mut start_is_sfb = 0;
        let mut in_is_block = 0;
        let mut current_is_sfb_count = 0;
        let mut overall_hrr_error: FixpDbl = 0;
        let mut is_region_loudness: FixpDbl = 0;

        for sfb in 0..max_sfb_per_group {
            let k = (sfboffs + sfb) as usize;
            if is_mask[k] == 1 {
                if current_is_sfb_count == 0 {
                    start_is_sfb = sfboffs + sfb;
                }
                if is_start_value_found == 0 {
                    is_scale_last = real_is_scale[k];
                    is_start_value_found = 1;
                }
                in_is_block = 1;
                current_is_sfb_count += 1;
                overall_hrr_error =
                    overall_hrr_error.wrapping_add(hrr_err[k] >> (MAX_SFB_PER_GROUP_SF - 3));
                is_region_loudness =
                    is_region_loudness.wrapping_add(norm_sfb_loudness[k] >> MAX_SFB_PER_GROUP_SF);
            } else {
                // based on correlation, IS should not be used
                // -> use it anyway, if overall error is below threshold
                //    and if local error does not exceed threshold
                // otherwise: check if there are enough IS SFBs
                if in_is_block != 0 {
                    overall_hrr_error =
                        overall_hrr_error.wrapping_add(hrr_err[k] >> (MAX_SFB_PER_GROUP_SF - 3));
                    is_region_loudness = is_region_loudness
                        .wrapping_add(norm_sfb_loudness[k] >> MAX_SFB_PER_GROUP_SF);

                    if (hrr_err[k] < (is_params.local_error_thresh >> 3))
                        && (overall_hrr_error
                            < (is_params.total_error_thresh >> MAX_SFB_PER_GROUP_SF))
                    {
                        current_is_sfb_count += 1;
                        // overwrite correlation based decision
                        is_mask[k] = 1;
                    } else {
                        in_is_block = 0;
                    }
                }
            }
            // check for large direction deviation
            if in_is_block != 0 {
                if f_abs(is_scale_last.wrapping_sub(real_is_scale[k]))
                    < (is_params.direction_deviation_thresh
                        >> (REAL_SCALE_SF + LD_DATA_SHIFT - IS_DIRECTION_DEVIATION_THRESH_SF))
                {
                    is_scale_last = real_is_scale[k];
                } else {
                    is_mask[k] = 0;
                    in_is_block = 0;
                    current_is_sfb_count -= 1;
                }
            }

            if current_is_sfb_count > 0 && (in_is_block == 0 || sfb == max_sfb_per_group - 1) {
                // not enough SFBs -> do not use IS
                if current_is_sfb_count < is_params.min_is_sfbs
                    || (is_region_loudness
                        < (is_params.is_region_min_loudness >> MAX_SFB_PER_GROUP_SF))
                {
                    for j in start_is_sfb..=sfboffs + sfb {
                        is_mask[j as usize] = 0;
                    }
                    is_scale_last = 0;
                    is_start_value_found = 0;
                    for j in 0..start_is_sfb {
                        if is_mask[j as usize] != 0 {
                            is_scale_last = real_is_scale[j as usize];
                            is_start_value_found = 1;
                        }
                    }
                }
                current_is_sfb_count = 0;
                overall_hrr_error = 0;
                is_region_loudness = 0;
            }
        }
        sfboffs += sfb_per_group;
    }
}

/// `FDKaacEnc_IntensityStereoProcessing(...)`: intensity stereo processing tool.
///
/// Outputs: `is_book`, `is_scale`, `pns_data[ch].pnsFlag`, `ms_digest`, `ms_mask` (zeroed from
/// start to sfbCnt); `mdct_spectrum_right`, `sfb_energy_right`, `sfb_spread_en_right`,
/// `sfb_threshold_right` zeroed where `isBook != 0`; `sfb_energy_ld_data_right`
/// `FL2FXCONST_DBL(-1.0)` and `sfb_threshold_ld_data_right` `FL2FXCONST_DBL(-0.515625f)` where
/// `isBook != 0`.
///
/// `pnsData[1]` is dereferenced in C whenever `pnsData[0]` is not `NULL`; the port skips a
/// `None` entry (the encoder always passes both).
#[allow(clippy::too_many_arguments)]
pub fn fdk_aac_enc_intensity_stereo_processing(
    sfb_energy_left: &[FixpDbl],
    sfb_energy_right: &mut [FixpDbl],
    mdct_spectrum_left: &mut [FixpDbl],
    mdct_spectrum_right: &mut [FixpDbl],
    sfb_threshold_left: &[FixpDbl],
    sfb_threshold_right: &mut [FixpDbl],
    sfb_threshold_ld_data_right: &mut [FixpDbl],
    _sfb_spread_en_left: &[FixpDbl],
    sfb_spread_en_right: &mut [FixpDbl],
    sfb_energy_ld_data_left: &[FixpDbl],
    sfb_energy_ld_data_right: &mut [FixpDbl],
    ms_digest: &mut i32,
    ms_mask: &mut [i32],
    sfb_cnt: i32,
    sfb_per_group: i32,
    max_sfb_per_group: i32,
    sfb_offset: &[i32],
    allow_is: i32,
    is_book: &mut [i32],
    is_scale: &mut [i32],
    mut pns_data: [Option<&mut PnsData>; 2],
) {
    let mut hrr_err = [0 as FixpDbl; MAX_GROUPED_SFB];
    let mut norm_sfb_loudness = [0 as FixpDbl; MAX_GROUPED_SFB];
    let mut real_is_scale = [0 as FixpDbl; MAX_GROUPED_SFB];
    let mut is_params = IntensityParameters::default();
    let mut is_mask = [0i32; MAX_GROUPED_SFB];

    let n = sfb_cnt.max(0) as usize;
    is_book[..n].fill(0);
    is_scale[..n].fill(0);

    if allow_is == 0 {
        return;
    }

    fdk_aac_enc_init_is_params(&mut is_params);

    // compute / set the following values per SFB:
    // - left/right ratio between channels
    // - normalized loudness
    //   + loudness == average of energy in channels to 0.25
    //   + normalization: division by sum of all SFB loudnesses
    // - isMask (is set to 0 if channels are the same or one is 0)
    fdk_aac_enc_prepare_intensity_decision(
        sfb_energy_left,
        sfb_energy_right,
        sfb_energy_ld_data_left,
        sfb_energy_ld_data_right,
        mdct_spectrum_left,
        mdct_spectrum_right,
        &is_params,
        &mut hrr_err,
        &mut is_mask,
        &mut real_is_scale,
        &mut norm_sfb_loudness,
        sfb_cnt,
        sfb_per_group,
        max_sfb_per_group,
        sfb_offset,
    );

    fdk_aac_enc_finalize_intensity_decision(
        &hrr_err,
        &mut is_mask,
        &real_is_scale,
        &norm_sfb_loudness,
        &is_params,
        sfb_cnt,
        sfb_per_group,
        max_sfb_per_group,
    );

    let mut sfb = 0;
    while sfb < sfb_cnt {
        for sfboffs in 0..max_sfb_per_group {
            let k = (sfb + sfboffs) as usize;
            let mut mdct_spec_sf = MDCT_SPEC_SF;

            ms_mask[k] = 0;
            if is_mask[k] == 0 {
                continue;
            }

            if (sfb_energy_left[k] < sfb_threshold_left[k])
                && (f_mult(fl2fxconst_dbl((1.0f32 / 1.5f32) as f64), sfb_energy_right[k])
                    > sfb_threshold_right[k])
            {
                continue;
            }
            // NEW: if there is a big-enough IS region, switch off PNS
            if pns_data[0].is_some() {
                for p in pns_data.iter_mut().flatten() {
                    if p.pns_flag[k] != 0 {
                        p.pns_flag[k] = 0;
                    }
                }
            }

            let (o0, o1) = (sfb_offset[k], sfb_offset[k + 1]);
            if o1 - o0 > 1 << mdct_spec_sf {
                // This is for rare cases where the number of bins in a scale factor band is > 64
                mdct_spec_sf += 1;
            }

            // scaled with 2 to compensate fMultDiv2() in subsequent loop
            let inv_n = get_inv_int((o1 - o0) >> 1);
            let s_l = calc_sfb_max_scale(mdct_spectrum_left, o0, o1);
            let s_r = calc_sfb_max_scale(mdct_spectrum_right, o0, o1);

            let mut lr: FixpDbl = 0;
            for j in o0..o1 {
                let j = j as usize;
                lr = lr.wrapping_add(f_mult_div2(
                    f_mult_div2(mdct_spectrum_left[j] << s_l, mdct_spectrum_right[j] << s_r),
                    inv_n,
                ));
            }
            lr <<= 1;

            if lr < 0 {
                // This means OUT OF phase intensity stereo, cf. standard
                let mut s1 = 0;
                let mut ed: FixpDbl = 0;

                let s0 = fix_min(s_l, s_r);
                for j in o0..o1 {
                    let j = j as usize;
                    let d = ((mdct_spectrum_left[j] << s0) >> 1)
                        - ((mdct_spectrum_right[j] << s0) >> 1);
                    ed = ed.wrapping_add(f_mult_div2(d, d) >> (mdct_spec_sf - 1));
                }
                ms_mask[k] = 1;
                let mut tmp = f_div_norm(sfb_energy_left[k], ed, &mut s1);
                let mut s2 = s1 + (2 * s0) - 2 - mdct_spec_sf;
                if s2 & 1 != 0 {
                    tmp >>= 1;
                    s2 += 1;
                }
                s2 = (s2 >> 1) + 1; // +1 compensate fMultDiv2() in subsequent loop
                s2 = fix_min(fix_max(s2, -(DFRACT_BITS - 1)), DFRACT_BITS - 1);
                let scale = sqrt_fixp(tmp);
                if s2 < 0 {
                    s2 = -s2;
                    for j in o0..o1 {
                        let j = j as usize;
                        mdct_spectrum_left[j] = (f_mult_div2(mdct_spectrum_left[j], scale)
                            - f_mult_div2(mdct_spectrum_right[j], scale))
                            >> s2;
                        mdct_spectrum_right[j] = 0;
                    }
                } else {
                    for j in o0..o1 {
                        let j = j as usize;
                        mdct_spectrum_left[j] = (f_mult_div2(mdct_spectrum_left[j], scale)
                            - f_mult_div2(mdct_spectrum_right[j], scale))
                            << s2;
                        mdct_spectrum_right[j] = 0;
                    }
                }
            } else {
                // This means IN phase intensity stereo, cf. standard
                let mut s1 = 0;
                let mut es: FixpDbl = 0;

                let s0 = fix_min(s_l, s_r);
                for j in o0..o1 {
                    let j = j as usize;
                    let s = ((mdct_spectrum_left[j] << s0) >> 1)
                        + ((mdct_spectrum_right[j] << s0) >> 1);
                    // scaled 2*(mdctScale - s0 + 1) + mdct_spec_sf
                    es = es.wrapping_add(f_mult_div2(s, s) >> (mdct_spec_sf - 1));
                }
                ms_mask[k] = 0;
                let mut tmp = f_div_norm(sfb_energy_left[k], es, &mut s1);
                let mut s2 = s1 + (2 * s0) - 2 - mdct_spec_sf;
                if s2 & 1 != 0 {
                    tmp >>= 1;
                    s2 += 1;
                }
                s2 = (s2 >> 1) + 1; // +1 compensate fMultDiv2() in subsequent loop
                s2 = fix_min(fix_max(s2, -(DFRACT_BITS - 1)), DFRACT_BITS - 1);
                let scale = sqrt_fixp(tmp);
                if s2 < 0 {
                    s2 = -s2;
                    for j in o0..o1 {
                        let j = j as usize;
                        mdct_spectrum_left[j] = (f_mult_div2(mdct_spectrum_left[j], scale)
                            + f_mult_div2(mdct_spectrum_right[j], scale))
                            >> s2;
                        mdct_spectrum_right[j] = 0;
                    }
                } else {
                    for j in o0..o1 {
                        let j = j as usize;
                        mdct_spectrum_left[j] = (f_mult_div2(mdct_spectrum_left[j], scale)
                            + f_mult_div2(mdct_spectrum_right[j], scale))
                            << s2;
                        mdct_spectrum_right[j] = 0;
                    }
                }
            }

            is_book[k] = CODE_BOOK_IS_IN_PHASE_NO;

            let half =
                fl2fxconst_dbl((0.5f32 / (1 << (REAL_SCALE_SF + LD_DATA_SHIFT + 1)) as f32) as f64);
            let sh = DFRACT_BITS - 1 - REAL_SCALE_SF - LD_DATA_SHIFT - 1;
            if real_is_scale[k] < 0 {
                is_scale[k] = (((real_is_scale[k] >> 1) - half) >> sh) + 1;
            } else {
                is_scale[k] = ((real_is_scale[k] >> 1) + half) >> sh;
            }

            sfb_energy_right[k] = 0;
            sfb_energy_ld_data_right[k] = fl2fxconst_dbl(-1.0);
            sfb_threshold_right[k] = 0;
            sfb_threshold_ld_data_right[k] = fl2fxconst_dbl(-0.515625f32 as f64);
            sfb_spread_en_right[k] = 0;

            *ms_digest = MS_SOME;
        }
        sfb += sfb_per_group;
    }
}

// The band loops of FDKaacEnc_prepareIntensityDecision (slices, sums and maxima in any order)
// against the straight loops (copied from the straight port), on random spectra and energies;
// intensity.rs of the cref tests compares the intensity stereo processing with C.
#[cfg(test)]
mod tests {
    use super::*;

    fn calc_sfb_max_scale_straight(mdct_spectrum: &[FixpDbl], l1: i32, l2: i32) -> i32 {
        let mut max_spc: FixpDbl = 0;
        for i in l1..l2 {
            let tmp = fixp_abs(mdct_spectrum[i as usize]);
            max_spc = fix_max(max_spc, tmp);
        }
        if max_spc == 0 {
            DFRACT_BITS - 2
        } else {
            cnt_leading_zeros(max_spc) - 1
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn fdk_aac_enc_prepare_intensity_decision_straight(
        sfb_energy_left: &[FixpDbl],
        sfb_energy_right: &[FixpDbl],
        sfb_energy_ld_data_left: &[FixpDbl],
        sfb_energy_ld_data_right: &[FixpDbl],
        mdct_spectrum_left: &[FixpDbl],
        mdct_spectrum_right: &[FixpDbl],
        is_params: &IntensityParameters,
        hrr_err: &mut [FixpDbl],
        is_mask: &mut [i32],
        real_scale: &mut [FixpDbl],
        norm_sfb_loudness: &mut [FixpDbl],
        sfb_cnt: i32,
        sfb_per_group: i32,
        max_sfb_per_group: i32,
        sfb_offset: &[i32],
    ) {
        // temporary variables to compute loudness
        let mut overall_loudness = [0 as FixpDbl; MAX_NO_OF_GROUPS];

        // temporary variables to compute correlation
        let mut channel_corr = [0 as FixpDbl; MAX_GROUPED_SFB];

        norm_sfb_loudness[..MAX_GROUPED_SFB].fill(0);
        real_scale[..MAX_GROUPED_SFB].fill(0);

        let mut grp_counter = 0usize;
        let mut sfboffs = 0;
        while sfboffs < sfb_cnt {
            overall_loudness[grp_counter] = 0;
            for sfb in 0..max_sfb_per_group {
                let k = (sfb + sfboffs) as usize;
                let is_value = sfb_energy_ld_data_left[k].wrapping_sub(sfb_energy_ld_data_right[k]);

                // delimitate intensity scale value to representable range
                real_scale[k] = fix_min(
                    fl2fxconst_dbl(
                        (60.0f32 / (1 << (REAL_SCALE_SF + LD_DATA_SHIFT)) as f32) as f64,
                    ),
                    fix_max(
                        fl2fxconst_dbl(
                            (-60.0f32 / (1 << (REAL_SCALE_SF + LD_DATA_SHIFT)) as f32) as f64,
                        ),
                        is_value,
                    ),
                );

                let s_l = fix_max(0, cnt_leading_zeros(sfb_energy_left[k]) - 1);
                let s_r = fix_max(0, cnt_leading_zeros(sfb_energy_right[k]) - 1);
                let s = (fix_min(s_l, s_r) >> 2) << 2;
                norm_sfb_loudness[k] = sqrt_fixp(sqrt_fixp(
                    ((sfb_energy_left[k] << s) >> 1) + ((sfb_energy_right[k] << s) >> 1),
                )) >> (s >> 2);

                overall_loudness[grp_counter] = overall_loudness[grp_counter]
                    .wrapping_add(norm_sfb_loudness[k] >> OVERALL_LOUDNESS_SF);
                // don't do intensity if
                // - panning angle is too close to the middle or
                // - one channel is non-existent or
                // - if it is dual mono
                if (sfb_energy_left[k]
                    >= f_mult(is_params.left_right_ratio_threshold, sfb_energy_right[k]))
                    && (f_mult(is_params.left_right_ratio_threshold, sfb_energy_left[k])
                        <= sfb_energy_right[k])
                {
                    // this will prevent post processing from considering this SFB for merging
                    hrr_err[k] = fl2fxconst_dbl(1.0 / 8.0);
                }
            }
            sfboffs += sfb_per_group;
            grp_counter += 1;
        }

        let mut grp_counter = 0usize;
        let mut sfboffs = 0;
        while sfboffs < sfb_cnt {
            let mut inv_overall_loudness_sf: i32;
            let inv_overall_loudness: FixpDbl;

            if overall_loudness[grp_counter] == 0 {
                inv_overall_loudness = 0;
                inv_overall_loudness_sf = 0;
            } else {
                inv_overall_loudness_sf = 0;
                inv_overall_loudness = f_div_norm(
                    MAXVAL_DBL,
                    overall_loudness[grp_counter],
                    &mut inv_overall_loudness_sf,
                );
                // +1: compensate fMultDiv2() in subsequent loop
                inv_overall_loudness_sf = inv_overall_loudness_sf - OVERALL_LOUDNESS_SF + 1;
            }
            inv_overall_loudness_sf =
                fix_min(fix_max(inv_overall_loudness_sf, -(DFRACT_BITS - 1)), DFRACT_BITS - 1);

            for sfb in 0..max_sfb_per_group {
                let k = (sfb + sfboffs) as usize;

                let tmp = f_mult_div2(
                    (norm_sfb_loudness[k] >> OVERALL_LOUDNESS_SF) << OVERALL_LOUDNESS_SF,
                    inv_overall_loudness,
                );

                norm_sfb_loudness[k] = scale_value(tmp, inv_overall_loudness_sf);

                channel_corr[k] = 0;

                // max width of scalefactorband is 96; width's are always even
                // inv_n is scaled with factor 2 to compensate fMultDiv2() in subsequent loops
                let inv_n = get_inv_int((sfb_offset[k + 1] - sfb_offset[k]) >> 1);

                if inv_n > 0 {
                    // correlation := Pearson's product-moment coefficient
                    // compute correlation between channels and check if it is over threshold
                    let mut ml: FixpDbl = 0;
                    let mut mr: FixpDbl = 0;
                    let mut prod_lr: FixpDbl = 0;
                    let mut square_l: FixpDbl = 0;
                    let mut square_r: FixpDbl = 0;

                    let mut s_l = calc_sfb_max_scale_straight(
                        mdct_spectrum_left,
                        sfb_offset[k],
                        sfb_offset[k + 1],
                    );
                    let mut s_r = calc_sfb_max_scale_straight(
                        mdct_spectrum_right,
                        sfb_offset[k],
                        sfb_offset[k + 1],
                    );
                    let mut s = fix_min(s_l, s_r);

                    for j in sfb_offset[k]..sfb_offset[k + 1] {
                        let j = j as usize;
                        // scaled with mdctScale - s + inv_n
                        ml = ml.wrapping_add(f_mult_div2(mdct_spectrum_left[j] << s, inv_n));
                        // scaled with mdctScale - s + inv_n
                        mr = mr.wrapping_add(f_mult_div2(mdct_spectrum_right[j] << s, inv_n));
                    }
                    ml = f_mult_div2(ml, inv_n); // scaled with mdctScale - s + inv_n
                    mr = f_mult_div2(mr, inv_n); // scaled with mdctScale - s + inv_n

                    for j in sfb_offset[k]..sfb_offset[k + 1] {
                        let j = j as usize;
                        // scaled with mdctScale - s + inv_n
                        let tmp_l = f_mult_div2(mdct_spectrum_left[j] << s, inv_n).wrapping_sub(ml);
                        // scaled with mdctScale - s + inv_n
                        let tmp_r =
                            f_mult_div2(mdct_spectrum_right[j] << s, inv_n).wrapping_sub(mr);

                        // scaled with 2*(mdctScale - s + inv_n) + 1
                        prod_lr = prod_lr.wrapping_add(f_mult_div2(tmp_l, tmp_r));
                        // scaled with 2*(mdctScale - s + inv_n) + 1
                        square_l = square_l.wrapping_add(f_pow2_div2(tmp_l));
                        // scaled with 2*(mdctScale - s + inv_n) + 1
                        square_r = square_r.wrapping_add(f_pow2_div2(tmp_r));
                    }
                    prod_lr <<= 1; // scaled with 2*(mdctScale - s + inv_n)
                    square_l <<= 1; // scaled with 2*(mdctScale - s + inv_n)
                    square_r <<= 1; // scaled with 2*(mdctScale - s + inv_n)

                    if square_l > 0 && square_r > 0 {
                        let mut channel_corr_sf = 0;

                        // local scaling of square_l and square_r is compensated after sqrt calculation
                        s_l = fix_max(0, cnt_leading_zeros(square_l) - 1);
                        s_r = fix_max(0, cnt_leading_zeros(square_r) - 1);
                        s = ((s_l + s_r) >> 1) << 1;
                        s_l = fix_min(s_l, s);
                        s_r = s - s_l;
                        let mut tmp = f_mult(square_l << s_l, square_r << s_r);
                        tmp = sqrt_fixp(tmp);

                        fdk_aac_rs_fdk::fdk_assert!(tmp > 0);

                        // numerator and denominator have the same scaling
                        if prod_lr < 0 {
                            channel_corr[k] =
                                f_div_norm(prod_lr.wrapping_neg(), tmp, &mut channel_corr_sf)
                                    .wrapping_neg();
                        } else {
                            channel_corr[k] = f_div_norm(prod_lr, tmp, &mut channel_corr_sf);
                        }
                        channel_corr_sf = fix_min(
                            fix_max(channel_corr_sf + ((s_l + s_r) >> 1), -(DFRACT_BITS - 1)),
                            DFRACT_BITS - 1,
                        );

                        if channel_corr_sf < 0 {
                            channel_corr[k] >>= -channel_corr_sf;
                        } else {
                            // avoid overflows due to limited computational accuracy
                            if f_abs(channel_corr[k]) > (MAXVAL_DBL >> channel_corr_sf) {
                                if channel_corr[k] < 0 {
                                    channel_corr[k] = -MAXVAL_DBL;
                                } else {
                                    channel_corr[k] = MAXVAL_DBL;
                                }
                            } else {
                                channel_corr[k] <<= channel_corr_sf;
                            }
                        }
                    }
                }

                // for post processing: hrrErr is the error in terms of (too little) correlation
                // weighted with the loudness of the SFB; SFBs with small hrrErr can be merged
                if hrr_err[k] == fl2fxconst_dbl(1.0 / 8.0) {
                    continue;
                }

                hrr_err[k] = f_mult_div2(
                    fl2fxconst_dbl(0.25f32 as f64).wrapping_sub(channel_corr[k] >> 2),
                    norm_sfb_loudness[k],
                );

                // set IS mask/vector to 1, if correlation is high enough
                if f_abs(channel_corr[k]) >= is_params.corr_thresh {
                    is_mask[k] = 1;
                }
            }
            sfboffs += sfb_per_group;
            grp_counter += 1;
        }
    }

    #[test]
    fn prepare_intensity_decision_vs_straight() {
        let mut seed = 0x5151_aaaa_1234_9876u64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed >> 32) as u32
        };
        let mut is_params = IntensityParameters {
            corr_thresh: 0,
            total_error_thresh: 0,
            local_error_thresh: 0,
            direction_deviation_thresh: 0,
            is_region_min_loudness: 0,
            min_is_sfbs: 0,
            left_right_ratio_threshold: 0,
        };
        fdk_aac_enc_init_is_params(&mut is_params);
        for _ in 0..5_000 {
            let per = 1 + (next() % 49) as i32;
            let groups = if next() % 3 == 0 { 1 + (next() % 4) as i32 } else { 1 };
            let sfb_cnt = (per * groups).min(MAX_GROUPED_SFB as i32);
            let per = sfb_cnt / groups;
            let max_sfb = (next() % (per as u32 + 1)) as i32;
            let mut offsets = vec![0i32; MAX_GROUPED_SFB + 1];
            for b in 1..offsets.len() {
                offsets[b] = (offsets[b - 1] + 2 * (next() % 12) as i32).min(1024);
            }
            let kind = next() % 3;
            let mut spec = || -> Vec<FixpDbl> {
                (0..1024)
                    .map(|_| match kind {
                        0 => (next() as i32) >> (next() % 32),
                        1 => [0, 1, -1, i32::MIN, i32::MAX][(next() % 5) as usize],
                        _ => (next() as i32) >> 8,
                    })
                    .collect()
            };
            let (l, r) = (spec(), spec());
            let mut en = || -> Vec<FixpDbl> {
                (0..MAX_GROUPED_SFB)
                    .map(|_| (next() >> (next() % 32)) as i32 & 0x7FFF_FFFF)
                    .collect()
            };
            let (el, er) = (en(), en());
            let (ell, elr) = (
                en().iter().map(|x| -x).collect::<Vec<_>>(),
                en().iter().map(|x| -x).collect::<Vec<_>>(),
            );
            let hrr0: Vec<FixpDbl> = (0..MAX_GROUPED_SFB).map(|_| next() as i32).collect();
            let mask0: Vec<i32> = (0..MAX_GROUPED_SFB).map(|_| (next() % 2) as i32).collect();
            let (mut ha, mut hb) = (hrr0.clone(), hrr0);
            let (mut ma, mut mb) = (mask0.clone(), mask0);
            let (mut ra, mut rb) = (vec![1; MAX_GROUPED_SFB], vec![1; MAX_GROUPED_SFB]);
            let (mut na, mut nb) = (vec![2; MAX_GROUPED_SFB], vec![2; MAX_GROUPED_SFB]);
            fdk_aac_enc_prepare_intensity_decision(
                &el, &er, &ell, &elr, &l, &r, &is_params, &mut ha, &mut ma, &mut ra, &mut na,
                sfb_cnt, per, max_sfb, &offsets,
            );
            fdk_aac_enc_prepare_intensity_decision_straight(
                &el, &er, &ell, &elr, &l, &r, &is_params, &mut hb, &mut mb, &mut rb, &mut nb,
                sfb_cnt, per, max_sfb, &offsets,
            );
            assert_eq!((&ha, &ma, &ra, &na), (&hb, &mb, &rb, &nb));
        }
    }
}
