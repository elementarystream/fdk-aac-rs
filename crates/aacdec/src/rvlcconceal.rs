// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACdec/src/rvlcconceal.h, libAACdec/src/rvlcconceal.cpp.

//! Port of `rvlcconceal.h`: the RVLC concealment of the scale factors.
//!
//! The functions reach `pComData->overlay.aac` (`erRvlcInfo`, `aRvlcScfFwd`, `aRvlcScfBwd`) and
//! take the channel's [`CAacDecoderCommonData`] as an additional last parameter `com` (see
//! [`crate::channelinfo`]). Indices outside the arrays are handled as described in
//! [`crate::rvlc`].

use fdk_aac_rs_fdk::common_fix::f_min;
use fdk_aac_rs_fdk::mdct::BLOCK_SHORT;

use crate::aacdec_hcr::{rd, wr};
use crate::channelinfo::{
    get_window_sequence, CAacDecoderChannelInfo, CAacDecoderCommonData,
    CAacDecoderStaticChannelInfo, INTENSITY_HCB, INTENSITY_HCB2, NOISE_HCB, ZERO_HCB,
};
use crate::rvlc_info::{CErRvlcInfo, CONCEAL_MAX_INIT, CONCEAL_MIN_INIT, SF_OFFSET};

/// `bnds` of `16 * group + band` as an index (negative values select no entry).
#[inline(always)]
fn ix(bnds: i32) -> usize {
    bnds as isize as usize
}

/// `calcRefValFwd(pRvlc, pAacDecoderChannelInfo, refIsFwd, refNrgFwd, refScfFwd)`: determines
/// the scale factor (and intensity position and noise energy) that is closest to the scale
/// factor band `conceal_min`, in forward direction.
///
/// `p_scf_fwd` is `pComData->overlay.aac.aRvlcScfFwd`.
pub fn calc_ref_val_fwd(
    p_rvlc: &CErRvlcInfo,
    p_aac_decoder_channel_info: &CAacDecoderChannelInfo,
    p_scf_fwd: &[i16],
    ref_is_fwd: &mut i32,
    ref_nrg_fwd: &mut i32,
    ref_scf_fwd: &mut i32,
) {
    let maximum_scale_factor_bands: i32 =
        if get_window_sequence(&p_aac_decoder_channel_info.ics_info) == BLOCK_SHORT {
            16
        } else {
            64
        };

    let conceal_min = p_rvlc.conceal_min as i32 % maximum_scale_factor_bands;
    let conceal_group_min = p_rvlc.conceal_min as i32 / maximum_scale_factor_bands;

    /* calculate first reference value for approach in forward direction */
    let (mut id_is, mut id_nrg, mut id_scf) = (1, 1, 1);

    let global_gain = p_aac_decoder_channel_info.p_dyn_data.raw_data_info.global_gain as i32;

    /* set reference values */
    *ref_is_fwd = -SF_OFFSET;
    *ref_nrg_fwd = global_gain - SF_OFFSET - 90 - 256;
    *ref_scf_fwd = global_gain - SF_OFFSET;

    let mut start_band = conceal_min - 1;
    let mut group = conceal_group_min;
    while group >= 0 {
        let mut band = start_band;
        while band >= 0 {
            let bnds = 16 * group + band;
            match rd(&p_aac_decoder_channel_info.p_dyn_data.a_code_book, ix(bnds)) {
                ZERO_HCB => {}
                INTENSITY_HCB | INTENSITY_HCB2 => {
                    if id_is != 0 {
                        *ref_is_fwd = rd(p_scf_fwd, ix(bnds)) as i32;
                        id_is = 0; /* reference value has been set */
                    }
                }
                NOISE_HCB => {
                    if id_nrg != 0 {
                        *ref_nrg_fwd = rd(p_scf_fwd, ix(bnds)) as i32;
                        id_nrg = 0; /* reference value has been set */
                    }
                }
                _ => {
                    if id_scf != 0 {
                        *ref_scf_fwd = rd(p_scf_fwd, ix(bnds)) as i32;
                        id_scf = 0; /* reference value has been set */
                    }
                }
            }
            band -= 1;
        }
        start_band = p_rvlc.max_sfb_transmitted as i32 - 1;
        group -= 1;
    }
}

/// `calcRefValBwd(pRvlc, pAacDecoderChannelInfo, refIsBwd, refNrgBwd, refScfBwd)`: determines
/// the scale factor (and intensity position and noise energy) that is closest to the scale
/// factor band `conceal_max`, in backward direction.
///
/// `p_scf_bwd` is `pComData->overlay.aac.aRvlcScfBwd`.
pub fn calc_ref_val_bwd(
    p_rvlc: &CErRvlcInfo,
    p_aac_decoder_channel_info: &CAacDecoderChannelInfo,
    p_scf_bwd: &[i16],
    ref_is_bwd: &mut i32,
    ref_nrg_bwd: &mut i32,
    ref_scf_bwd: &mut i32,
) {
    let maximum_scale_factor_bands: i32 =
        if get_window_sequence(&p_aac_decoder_channel_info.ics_info) == BLOCK_SHORT {
            16
        } else {
            64
        };

    let conceal_max = p_rvlc.conceal_max as i32 % maximum_scale_factor_bands;
    let conceal_group_max = p_rvlc.conceal_max as i32 / maximum_scale_factor_bands;

    /* calculate first reference value for approach in backward direction */
    let (mut id_is, mut id_nrg, mut id_scf) = (1, 1, 1);

    /* set reference values */
    *ref_is_bwd = p_rvlc.dpcm_is_last_position - SF_OFFSET;
    *ref_nrg_bwd = p_rvlc.rev_global_gain + p_rvlc.dpcm_noise_last_position - SF_OFFSET - 90 - 256
        + p_rvlc.dpcm_noise_nrg;
    *ref_scf_bwd = p_rvlc.rev_global_gain - SF_OFFSET;

    let mut start_band = conceal_max + 1;

    /* if needed, re-set reference values */
    let mut group = conceal_group_max;
    while group < p_rvlc.num_window_groups as i32 {
        let mut band = start_band;
        while band < p_rvlc.max_sfb_transmitted as i32 {
            let bnds = 16 * group + band;
            match rd(&p_aac_decoder_channel_info.p_dyn_data.a_code_book, ix(bnds)) {
                ZERO_HCB => {}
                INTENSITY_HCB | INTENSITY_HCB2 => {
                    if id_is != 0 {
                        *ref_is_bwd = rd(p_scf_bwd, ix(bnds)) as i32;
                        id_is = 0; /* reference value has been set */
                    }
                }
                NOISE_HCB => {
                    if id_nrg != 0 {
                        *ref_nrg_bwd = rd(p_scf_bwd, ix(bnds)) as i32;
                        id_nrg = 0; /* reference value has been set */
                    }
                }
                _ => {
                    if id_scf != 0 {
                        *ref_scf_bwd = rd(p_scf_bwd, ix(bnds)) as i32;
                        id_scf = 0; /* reference value has been set */
                    }
                }
            }
            band += 1;
        }
        start_band = 0;
        group += 1;
    }
}

/// The start of both bidirectional estimations: replaces `conceal_min` and `conceal_max` at
/// their initial values by the first and the last band, and returns `MaximumScaleFactorBands`.
fn conceal_borders(
    p_rvlc: &mut CErRvlcInfo,
    p_aac_decoder_channel_info: &CAacDecoderChannelInfo,
) -> i32 {
    let maximum_scale_factor_bands: i32 =
        if get_window_sequence(&p_aac_decoder_channel_info.ics_info) == BLOCK_SHORT {
            16
        } else {
            64
        };

    /* If an error was detected just in forward or backward direction, set the corresponding
    border for concealment to a appropriate scalefactor band. The border is set to first or last
    sfb respectively, because the error will possibly not follow directly after the corrupt bit
    but just after decoding some more (wrong) scalefactors. */
    if p_rvlc.conceal_min as i32 == CONCEAL_MIN_INIT {
        p_rvlc.conceal_min = 0;
    }

    if p_rvlc.conceal_max as i32 == CONCEAL_MAX_INIT {
        p_rvlc.conceal_max = ((p_rvlc.num_window_groups as i32 - 1) * 16
            + p_rvlc.max_sfb_transmitted as i32
            - 1) as i16;
    }

    maximum_scale_factor_bands
}

/// The end of both bidirectional estimations: copies the scale factors that need not be
/// concealed (forward decoded ones before `conceal_min`, backward decoded ones after
/// `conceal_max`).
fn copy_unconcealed(
    p_rvlc: &CErRvlcInfo,
    a_scale_factor: &mut [i16],
    p_scf_fwd: &[i16],
    p_scf_bwd: &[i16],
    conceal_min: i32,
    conceal_group_min: i32,
    conceal_max: i32,
    conceal_group_max: i32,
) {
    /* now copy all data to the output buffer which needs not to be concealed */
    let mut end_band =
        if conceal_group_min == 0 { conceal_min } else { p_rvlc.max_sfb_transmitted as i32 };
    let mut group = 0;
    while group <= conceal_group_min {
        let mut band = 0;
        while band < end_band {
            let bnds = 16 * group + band;
            wr(a_scale_factor, ix(bnds), rd(p_scf_fwd, ix(bnds)));
            band += 1;
        }
        if (group + 1) == conceal_group_min {
            end_band = conceal_min;
        }
        group += 1;
    }

    let mut start_band = conceal_max + 1;
    let mut group = conceal_group_max;
    while group < p_rvlc.num_window_groups as i32 {
        let mut band = start_band;
        while band < p_rvlc.max_sfb_transmitted as i32 {
            let bnds = 16 * group + band;
            wr(a_scale_factor, ix(bnds), rd(p_scf_bwd, ix(bnds)));
            band += 1;
        }
        start_band = 0;
        group += 1;
    }
}

/// `BidirectionalEstimation_UseLowerScfOfCurrentFrame(pAacDecoderChannelInfo)`: bidirectional
/// estimation when a single bit error was detected and isolated between `conceal_min` and
/// `conceal_max` and the `sf_concealment` flag is not set: the smaller of the forward and the
/// backward decoded scale factor (intensity position, noise energy) is taken.
///
/// `com` is the channel's `pComData`.
pub fn bidirectional_estimation_use_lower_scf_of_current_frame(
    p_aac_decoder_channel_info: &mut CAacDecoderChannelInfo,
    com: &mut CAacDecoderCommonData,
) {
    let aac = &mut com.overlay.aac;
    let p_rvlc = &mut aac.er_rvlc_info;

    let maximum_scale_factor_bands = conceal_borders(p_rvlc, p_aac_decoder_channel_info);

    let conceal_min = p_rvlc.conceal_min as i32 % maximum_scale_factor_bands;
    let conceal_group_min = p_rvlc.conceal_min as i32 / maximum_scale_factor_bands;
    let conceal_max = p_rvlc.conceal_max as i32 % maximum_scale_factor_bands;
    let conceal_group_max = p_rvlc.conceal_max as i32 / maximum_scale_factor_bands;

    if p_rvlc.conceal_min == p_rvlc.conceal_max {
        let (mut ref_is_fwd, mut ref_nrg_fwd, mut ref_scf_fwd) = (0, 0, 0);
        let (mut ref_is_bwd, mut ref_nrg_bwd, mut ref_scf_bwd) = (0, 0, 0);

        let bnds = p_rvlc.conceal_min as i32;
        calc_ref_val_fwd(
            p_rvlc,
            p_aac_decoder_channel_info,
            &aac.a_rvlc_scf_fwd,
            &mut ref_is_fwd,
            &mut ref_nrg_fwd,
            &mut ref_scf_fwd,
        );
        calc_ref_val_bwd(
            p_rvlc,
            p_aac_decoder_channel_info,
            &aac.a_rvlc_scf_bwd,
            &mut ref_is_bwd,
            &mut ref_nrg_bwd,
            &mut ref_scf_bwd,
        );

        let p_dyn_data = &mut *p_aac_decoder_channel_info.p_dyn_data;
        match rd(&p_dyn_data.a_code_book, ix(bnds)) {
            ZERO_HCB => {}
            INTENSITY_HCB | INTENSITY_HCB2 => {
                let v = if ref_is_fwd < ref_is_bwd { ref_is_fwd } else { ref_is_bwd };
                wr(&mut p_dyn_data.a_scale_factor, ix(bnds), v as i16);
            }
            NOISE_HCB => {
                let v = if ref_nrg_fwd < ref_nrg_bwd { ref_nrg_fwd } else { ref_nrg_bwd };
                wr(&mut p_dyn_data.a_scale_factor, ix(bnds), v as i16);
            }
            _ => {
                let v = if ref_scf_fwd < ref_scf_bwd { ref_scf_fwd } else { ref_scf_bwd };
                wr(&mut p_dyn_data.a_scale_factor, ix(bnds), v as i16);
            }
        }
    } else {
        let cmax = p_rvlc.conceal_max as isize as usize;
        let cmin = p_rvlc.conceal_min as isize as usize;
        let v = rd(&aac.a_rvlc_scf_bwd, cmax);
        wr(&mut aac.a_rvlc_scf_fwd, cmax, v);
        let v = rd(&aac.a_rvlc_scf_fwd, cmin);
        wr(&mut aac.a_rvlc_scf_bwd, cmin, v);

        /* consider the smaller of the forward and backward decoded value as the correct one */
        let p_dyn_data = &mut *p_aac_decoder_channel_info.p_dyn_data;
        let mut start_band = conceal_min;
        let mut end_band = if conceal_group_min == conceal_group_max {
            conceal_max
        } else {
            p_rvlc.max_sfb_transmitted as i32 - 1
        };

        let mut group = conceal_group_min;
        while group <= conceal_group_max {
            let mut band = start_band;
            while band <= end_band {
                let bnds = 16 * group + band;
                let f = rd(&aac.a_rvlc_scf_fwd, ix(bnds));
                let b = rd(&aac.a_rvlc_scf_bwd, ix(bnds));
                wr(&mut p_dyn_data.a_scale_factor, ix(bnds), if f < b { f } else { b });
                band += 1;
            }
            start_band = 0;
            if (group + 1) == conceal_group_max {
                end_band = conceal_max;
            }
            group += 1;
        }
    }

    copy_unconcealed(
        &aac.er_rvlc_info,
        &mut p_aac_decoder_channel_info.p_dyn_data.a_scale_factor,
        &aac.a_rvlc_scf_fwd,
        &aac.a_rvlc_scf_bwd,
        conceal_min,
        conceal_group_min,
        conceal_max,
        conceal_group_max,
    );
}

/// `fMin(commonMin, aRvlcPreviousScaleFactor[bnds])` or `fMin(fwd, bwd)`: the scale factor of
/// band `bnds` for the estimations that use the previous frame, depending on whether the previous
/// frame has a codebook of the same kind (`same_kind`).
#[inline]
fn min_with_prev(fwd: i16, bwd: i16, prev: i16, same_kind: bool) -> i16 {
    if same_kind {
        let common_min: i16 = f_min(fwd, bwd);
        f_min(common_min, prev)
    } else {
        f_min(fwd, bwd)
    }
}

/// `BidirectionalEstimation_UseScfOfPrevFrameAsReference(pAacDecoderChannelInfo,
/// pAacDecoderStaticChannelInfo)`: bidirectional estimation when a single bit error was detected
/// and isolated between `conceal_min` and `conceal_max`, the `sf_concealment` flag is set and the
/// previous frame has the same block type: the smallest of the forward decoded, the backward
/// decoded and the previous frame's scale factor is taken (the previous one only if its codebook
/// is of the same kind).
///
/// `com` is the channel's `pComData`.
pub fn bidirectional_estimation_use_scf_of_prev_frame_as_reference(
    p_aac_decoder_channel_info: &mut CAacDecoderChannelInfo,
    p_aac_decoder_static_channel_info: &mut CAacDecoderStaticChannelInfo,
    com: &mut CAacDecoderCommonData,
) {
    let aac = &mut com.overlay.aac;
    let p_rvlc = &mut aac.er_rvlc_info;

    let maximum_scale_factor_bands = conceal_borders(p_rvlc, p_aac_decoder_channel_info);

    let conceal_min = p_rvlc.conceal_min as i32 % maximum_scale_factor_bands;
    let conceal_group_min = p_rvlc.conceal_min as i32 / maximum_scale_factor_bands;
    let conceal_max = p_rvlc.conceal_max as i32 % maximum_scale_factor_bands;
    let conceal_group_max = p_rvlc.conceal_max as i32 / maximum_scale_factor_bands;

    let cmax = p_rvlc.conceal_max as isize as usize;
    let cmin = p_rvlc.conceal_min as isize as usize;
    let v = rd(&aac.a_rvlc_scf_bwd, cmax);
    wr(&mut aac.a_rvlc_scf_fwd, cmax, v);
    let v = rd(&aac.a_rvlc_scf_fwd, cmin);
    wr(&mut aac.a_rvlc_scf_bwd, cmin, v);

    /* consider the smaller of the forward and backward decoded value as the correct one */
    let ci = &p_aac_decoder_static_channel_info.concealment_info;
    let p_dyn_data = &mut *p_aac_decoder_channel_info.p_dyn_data;
    let mut start_band = conceal_min;
    let mut end_band = if conceal_group_min == conceal_group_max {
        conceal_max
    } else {
        p_rvlc.max_sfb_transmitted as i32 - 1
    };

    let mut group = conceal_group_min;
    while group <= conceal_group_max {
        let mut band = start_band;
        while band <= end_band {
            let bnds = 16 * group + band;
            let i = ix(bnds);
            let fwd = rd(&aac.a_rvlc_scf_fwd, i);
            let bwd = rd(&aac.a_rvlc_scf_bwd, i);
            let prev = rd(&ci.a_rvlc_previous_scale_factor, i);
            let prev_cb = rd(&ci.a_rvlc_previous_codebook, i);
            match rd(&p_dyn_data.a_code_book, i) {
                ZERO_HCB => {
                    wr(&mut p_dyn_data.a_scale_factor, i, 0);
                }

                INTENSITY_HCB | INTENSITY_HCB2 => {
                    let same = (prev_cb == INTENSITY_HCB) || (prev_cb == INTENSITY_HCB2);
                    wr(&mut p_dyn_data.a_scale_factor, i, min_with_prev(fwd, bwd, prev, same));
                }

                NOISE_HCB => {
                    let same = prev_cb == NOISE_HCB;
                    wr(&mut p_dyn_data.a_scale_factor, i, min_with_prev(fwd, bwd, prev, same));
                }

                _ => {
                    let same = (prev_cb != ZERO_HCB)
                        && (prev_cb != NOISE_HCB)
                        && (prev_cb != INTENSITY_HCB)
                        && (prev_cb != INTENSITY_HCB2);
                    wr(&mut p_dyn_data.a_scale_factor, i, min_with_prev(fwd, bwd, prev, same));
                }
            }
            band += 1;
        }
        start_band = 0;
        if (group + 1) == conceal_group_max {
            end_band = conceal_max;
        }
        group += 1;
    }

    copy_unconcealed(
        &aac.er_rvlc_info,
        &mut p_dyn_data.a_scale_factor,
        &aac.a_rvlc_scf_fwd,
        &aac.a_rvlc_scf_bwd,
        conceal_min,
        conceal_group_min,
        conceal_max,
        conceal_group_max,
    );
}

/// `StatisticalEstimation(pAacDecoderChannelInfo)`: statistical estimation when the start and the
/// end values differ and no further errors were detected: for scale factors, intensity
/// positions and noise energies separately, the direction (forward or backward) with the lower
/// sum is taken.
///
/// `com` is the channel's `pComData`.
pub fn statistical_estimation(
    p_aac_decoder_channel_info: &mut CAacDecoderChannelInfo,
    com: &mut CAacDecoderCommonData,
) {
    let aac = &com.overlay.aac;
    let p_rvlc = &aac.er_rvlc_info;
    let p_dyn_data = &mut *p_aac_decoder_channel_info.p_dyn_data;
    /* sum of intensity data forward/backward */
    let (mut sum_is_fwd, mut sum_is_bwd): (i32, i32) = (0, 0);
    /* sum of noise energy data forward/backward */
    let (mut sum_nrg_fwd, mut sum_nrg_bwd): (i32, i32) = (0, 0);
    /* sum of scalefactor data forward/backward */
    let (mut sum_scf_fwd, mut sum_scf_bwd): (i32, i32) = (0, 0);

    /* calculate sum of each group (scf,nrg,is) of forward and backward direction */
    for group in 0..p_rvlc.num_window_groups as usize {
        for band in 0..p_rvlc.max_sfb_transmitted as usize {
            let bnds = 16 * group + band;
            let f = rd(&aac.a_rvlc_scf_fwd, bnds) as i32;
            let b = rd(&aac.a_rvlc_scf_bwd, bnds) as i32;
            match rd(&p_dyn_data.a_code_book, bnds) {
                ZERO_HCB => {}

                INTENSITY_HCB | INTENSITY_HCB2 => {
                    sum_is_fwd += f;
                    sum_is_bwd += b;
                }

                NOISE_HCB => {
                    sum_nrg_fwd += f;
                    sum_nrg_bwd += b;
                }

                _ => {
                    sum_scf_fwd += f;
                    sum_scf_bwd += b;
                }
            }
        }
    }

    /* find for each group (scf,nrg,is) the correct direction */
    let use_is_fwd = sum_is_fwd < sum_is_bwd;
    let use_nrg_fwd = sum_nrg_fwd < sum_nrg_bwd;
    let use_scf_fwd = sum_scf_fwd < sum_scf_bwd;

    /* conceal each group (scf,nrg,is) */
    for group in 0..p_rvlc.num_window_groups as usize {
        for band in 0..p_rvlc.max_sfb_transmitted as usize {
            let bnds = 16 * group + band;
            let f = rd(&aac.a_rvlc_scf_fwd, bnds);
            let b = rd(&aac.a_rvlc_scf_bwd, bnds);
            match rd(&p_dyn_data.a_code_book, bnds) {
                ZERO_HCB => {}

                INTENSITY_HCB | INTENSITY_HCB2 => {
                    wr(&mut p_dyn_data.a_scale_factor, bnds, if use_is_fwd { f } else { b });
                }

                NOISE_HCB => {
                    wr(&mut p_dyn_data.a_scale_factor, bnds, if use_nrg_fwd { f } else { b });
                }

                _ => {
                    wr(&mut p_dyn_data.a_scale_factor, bnds, if use_scf_fwd { f } else { b });
                }
            }
        }
    }
}

/// `PredictiveInterpolation(pAacDecoderChannelInfo, pAacDecoderStaticChannelInfo)`: predictive
/// estimation when the error cannot be isolated between `conceal_min` and `conceal_max`, the
/// `sf_concealment` flag is set and the previous frame has the same block type: for each band
/// with the same kind of data in the previous frame, the smallest of the forward, backward and
/// previous value is taken, otherwise a fixed value (0 for scale factors, -110 for intensity
/// positions and noise energies).
///
/// `com` is the channel's `pComData`.
pub fn predictive_interpolation(
    p_aac_decoder_channel_info: &mut CAacDecoderChannelInfo,
    p_aac_decoder_static_channel_info: &mut CAacDecoderStaticChannelInfo,
    com: &mut CAacDecoderCommonData,
) {
    let aac = &com.overlay.aac;
    let p_rvlc = &aac.er_rvlc_info;
    let ci = &p_aac_decoder_static_channel_info.concealment_info;
    let p_dyn_data = &mut *p_aac_decoder_channel_info.p_dyn_data;

    for group in 0..p_rvlc.num_window_groups as usize {
        for band in 0..p_rvlc.max_sfb_transmitted as usize {
            let bnds = 16 * group + band;
            let fwd = rd(&aac.a_rvlc_scf_fwd, bnds);
            let bwd = rd(&aac.a_rvlc_scf_bwd, bnds);
            let prev = rd(&ci.a_rvlc_previous_scale_factor, bnds);
            let prev_cb = rd(&ci.a_rvlc_previous_codebook, bnds);
            match rd(&p_dyn_data.a_code_book, bnds) {
                ZERO_HCB => {
                    wr(&mut p_dyn_data.a_scale_factor, bnds, 0);
                }

                INTENSITY_HCB | INTENSITY_HCB2 => {
                    if (prev_cb == INTENSITY_HCB) || (prev_cb == INTENSITY_HCB2) {
                        let common_min: i16 = f_min(fwd, bwd);
                        wr(&mut p_dyn_data.a_scale_factor, bnds, f_min(common_min, prev));
                    } else {
                        wr(&mut p_dyn_data.a_scale_factor, bnds, -110);
                    }
                }

                NOISE_HCB => {
                    if prev_cb == NOISE_HCB {
                        let common_min: i16 = f_min(fwd, bwd);
                        wr(&mut p_dyn_data.a_scale_factor, bnds, f_min(common_min, prev));
                    } else {
                        wr(&mut p_dyn_data.a_scale_factor, bnds, -110);
                    }
                }

                _ => {
                    if (prev_cb != ZERO_HCB)
                        && (prev_cb != NOISE_HCB)
                        && (prev_cb != INTENSITY_HCB)
                        && (prev_cb != INTENSITY_HCB2)
                    {
                        let common_min: i16 = f_min(fwd, bwd);
                        wr(&mut p_dyn_data.a_scale_factor, bnds, f_min(common_min, prev));
                    } else {
                        wr(&mut p_dyn_data.a_scale_factor, bnds, 0);
                    }
                }
            }
        }
    }
}
