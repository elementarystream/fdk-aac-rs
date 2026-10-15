// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACdec/src/stereo.h, libAACdec/src/stereo.cpp.

//! Port of `stereo.h`: joint stereo processing (M/S, intensity stereo, complex stereo
//! prediction).
//!
//! The types and constants are defined in [`crate::channelinfo::stereo_types`] and
//! re-exported here. `CJointStereo_ApplyMS()` and `CJointStereo_ApplyIS()` are declared in
//! `channelinfo.h` and defined in `stereo.cpp`.
//!
//! # Memory
//!
//! `CChannelElement_Decode()` is the only caller of the apply functions. The pointer parameters
//! of `CJointStereo_ApplyMS()` that always point to fields of the two channels or of the
//! element's `pComData` (`spectrumL/R`, `SFBleftScale/SFBrightScale`, `specScaleL/R`,
//! `store_dmx_re_prev(_e)`) are dropped: the port takes them from those structures. The memory
//! of the pointers of `CJointStereoPersistentData` is passed as described in
//! [`crate::channelinfo::stereo_types`].
//!
//! The `.ii` files of the `aarch64` and `x86` builds differ only in the expansion of
//! `SATURATE_SHIFT` (`arm/scale_arm.h` versus the generic `scale.h`), which
//! [`saturate_shift`] implements per variant, in the expansion of `FDK_ASSERT` and in the
//! shared header primitives.

pub use crate::channelinfo::stereo_types::*;

use fdk_aac_rs_fdk::audio::{AC_INDEP, AC_RSV603DA, AC_RSVD50, AC_USAC};
use fdk_aac_rs_fdk::bitstream::{fdk_read_bits, FdkBitStream};
use fdk_aac_rs_fdk::common_fix::{
    count_leading_bits, f_mult, f_mult_add_div2, f_mult_div2, saturate_left_shift_alt,
    saturate_shift, scale_values, scale_values_to, FixpDbl, FixpSgl, DFRACT_BITS,
};
use fdk_aac_rs_fdk::fdk_assert;
use fdk_aac_rs_fdk::mdct::{
    BLOCK_LONG, BLOCK_SHORT, BLOCK_START, BLOCK_STOP, SHAPE_KBD, SHAPE_SINE,
};

use crate::aac_rom::{
    FixpFilt, AAC_CODE_BOOK_DESCRIPTION_TABLE, MANTISSA_TABLE, MDST_FILT_COEF_CURR,
    MDST_FILT_COEF_PREV, SF_FNA_COEFFS,
};
use crate::block::cblock_decode_huffman_word;
use crate::channelinfo::{
    get_window_sequence, get_window_shape, CAacDecoderChannelInfo, CAacDecoderCommonData,
    CAacDecoderCommonStaticData, CAacDecoderStaticChannelInfo, CpePersistentData, BOOKSCL,
    INTENSITY_HCB, INTENSITY_HCB2,
};
use crate::overlapadd::spec as spec_off;

/// `L`
const L: usize = 0;
/// `R`
const R: usize = 1;

/// `CJointStereo_Read(bs, pJointStereoData, windowGroups, scaleFactorBandsTransmitted,
/// max_sfb_ste_clear, pJointStereoPersistentData, cplxPredictionData, cplxPredictionActiv,
/// scaleFactorBandsTotal, windowSequence, flags)`: read the joint stereo data. Returns 0 on
/// success, -1 on error.
///
/// `p_joint_stereo_persistent_data` and `cplx_prediction_data` are `None` for `NULL`; C passes
/// `NULL` only with `cplxPredictionActiv == 0`. With `cplxPredictionActiv != 0` and `None` the
/// reference has no defined result; the port uses a zero-initialized structure that is
/// discarded afterwards.
pub fn cjoint_stereo_read(
    bs: &mut FdkBitStream,
    p_joint_stereo_data: &mut CJointStereoData,
    window_groups: i32,
    scale_factor_bands_transmitted: i32,
    max_sfb_ste_clear: i32,
    p_joint_stereo_persistent_data: Option<&mut CJointStereoPersistentData>,
    cplx_prediction_data: Option<&mut CCplxPredictionData>,
    cplx_prediction_activ: i32,
    scale_factor_bands_total: i32,
    window_sequence: i32,
    flags: u32,
) -> i32 {
    let mut dummy_persistent;
    let pjs: &mut CJointStereoPersistentData = match p_joint_stereo_persistent_data {
        Some(p) => p,
        None => {
            dummy_persistent = CJointStereoPersistentData::default();
            &mut dummy_persistent
        }
    };
    let mut dummy_cplx;
    let cplx: &mut CCplxPredictionData = match cplx_prediction_data {
        Some(p) => p,
        None => {
            dummy_cplx = CCplxPredictionData::default();
            &mut dummy_cplx
        }
    };
    let jsd = p_joint_stereo_data;

    jsd.ms_mask_present = fdk_read_bits(bs, 2) as u8;

    let nclear = (scale_factor_bands_transmitted.max(0) as usize).min(jsd.ms_used.len());
    jsd.ms_used[..nclear].fill(0);

    // MsUsed[band] for a band outside the array (more than 64 bands) is skipped.
    fn ms_or(ms_used: &mut [u8], band: i32, v: u8) {
        if let Some(m) = ms_used.get_mut(band as usize) {
            *m |= v;
        }
    }
    fn ms_get(ms_used: &[u8], band: i32) -> u8 {
        ms_used.get(band as usize).copied().unwrap_or(0)
    }

    jsd.cplx_pred_flag = 0;
    if cplx_prediction_activ != 0 {
        cplx.pred_dir = 0;
        cplx.complex_coef = 0;
        cplx.use_prev_frame = 0;
        cplx.igf_pred_dir = 0;
    }

    match jsd.ms_mask_present {
        0 => { /* no M/S */ /* all flags are already cleared */ }
        1 => {
            /* read ms_used */
            for group in 0..window_groups {
                for band in 0..scale_factor_bands_transmitted {
                    ms_or(&mut jsd.ms_used, band, ((fdk_read_bits(bs, 1) as i32) << group) as u8);
                }
            }
        }
        2 => {
            /* full spectrum M/S */
            for band in 0..scale_factor_bands_transmitted {
                if let Some(m) = jsd.ms_used.get_mut(band as usize) {
                    *m = 255; /* set all flags to 1 */
                }
            }
        }
        _ => {
            /* M/S coding is disabled, complex stereo prediction is enabled */
            if flags & (AC_USAC | AC_RSVD50 | AC_RSV603DA) != 0 {
                if cplx_prediction_activ != 0 {
                    /* 'if (stereoConfigIndex == 0)' */

                    jsd.cplx_pred_flag = 1;

                    /* cplx_pred_data()  cp. ISO/IEC FDIS 23003-3:2011(E)  Table 26 */
                    let cplx_pred_all = fdk_read_bits(bs, 1);

                    if cplx_pred_all != 0 {
                        for group in 0..window_groups {
                            let groupmask = (1u8 as i32).wrapping_shl(group as u32) as u8;
                            for band in 0..scale_factor_bands_transmitted {
                                ms_or(&mut jsd.ms_used, band, groupmask);
                            }
                        }
                    } else {
                        for group in 0..window_groups {
                            let mut band = 0;
                            while band < scale_factor_bands_transmitted {
                                ms_or(
                                    &mut jsd.ms_used,
                                    band,
                                    ((fdk_read_bits(bs, 1) as i32) << group) as u8,
                                );
                                if (band + 1) < scale_factor_bands_total {
                                    let v = ms_get(&jsd.ms_used, band)
                                        & (1u8 as i32).wrapping_shl(group as u32) as u8;
                                    ms_or(&mut jsd.ms_used, band + 1, v);
                                }
                                band += SFB_PER_PRED_BAND;
                            }
                        }
                    }
                } else {
                    return -1;
                }
            }
        }
    }

    if cplx_prediction_activ != 0 {
        /* If all sfb are MS-ed then no complex prediction */
        if jsd.ms_mask_present == 3 {
            if jsd.cplx_pred_flag != 0 {
                let delta_code_time;

                /* set pointer to Huffman codebooks */
                let hcb = &AAC_CODE_BOOK_DESCRIPTION_TABLE[BOOKSCL as usize];
                /* set predictors to zero in case of a transition from long to short window
                 * sequences and vice versa */
                if ((window_sequence == BLOCK_SHORT) && (pjs.win_seq_prev as i32 != BLOCK_SHORT))
                    || ((pjs.win_seq_prev as i32 == BLOCK_SHORT)
                        && (window_sequence != BLOCK_SHORT))
                {
                    pjs.alpha_q_re_prev =
                        [[0; JointStereoMaximumBands as usize]; JointStereoMaximumGroups as usize];
                    pjs.alpha_q_im_prev =
                        [[0; JointStereoMaximumBands as usize]; JointStereoMaximumGroups as usize];
                }
                {
                    cplx.alpha_q_re =
                        [[0; JointStereoMaximumBands as usize]; JointStereoMaximumGroups as usize];
                    cplx.alpha_q_im =
                        [[0; JointStereoMaximumBands as usize]; JointStereoMaximumGroups as usize];
                }

                /* 0 = mid->side prediction, 1 = side->mid prediction */
                cplx.pred_dir = fdk_read_bits(bs, 1) as u8;
                cplx.complex_coef = fdk_read_bits(bs, 1) as u8;

                if cplx.complex_coef != 0 {
                    if flags & AC_INDEP != 0 {
                        cplx.use_prev_frame = 0;
                    } else {
                        cplx.use_prev_frame = fdk_read_bits(bs, 1) as u8;
                    }
                }

                if flags & AC_INDEP != 0 {
                    delta_code_time = 0;
                } else {
                    delta_code_time = fdk_read_bits(bs, 1);
                }

                {
                    let mut last_alpha_q_re: i32;
                    let mut last_alpha_q_im: i32;

                    // Groups and bands outside the arrays (inconsistent parameters) are
                    // skipped.
                    let gmax = window_groups.clamp(0, JointStereoMaximumGroups);
                    let bmax = JointStereoMaximumBands;
                    for group in 0..gmax {
                        let g = group as usize;
                        let mut band = 0;
                        while band < scale_factor_bands_transmitted {
                            if band >= bmax {
                                break;
                            }
                            let b = band as usize;
                            if delta_code_time == 1 {
                                if group > 0 {
                                    last_alpha_q_re = cplx.alpha_q_re[g - 1][b] as i32;
                                    last_alpha_q_im = cplx.alpha_q_im[g - 1][b] as i32;
                                } else if (window_sequence == BLOCK_SHORT)
                                    && (pjs.win_seq_prev as i32 == BLOCK_SHORT)
                                {
                                    /* Included for error-robustness */
                                    if pjs.win_groups_prev == 0 {
                                        return -1;
                                    }

                                    let gp = (pjs.win_groups_prev as usize - 1)
                                        .min(JointStereoMaximumGroups as usize - 1);
                                    last_alpha_q_re = pjs.alpha_q_re_prev[gp][b] as i32;
                                    last_alpha_q_im = pjs.alpha_q_im_prev[gp][b] as i32;
                                } else {
                                    last_alpha_q_re = pjs.alpha_q_re_prev[g][b] as i32;
                                    last_alpha_q_im = pjs.alpha_q_im_prev[g][b] as i32;
                                }
                            } else {
                                if band > 0 {
                                    last_alpha_q_re = cplx.alpha_q_re[g][b - 1] as i32;
                                    last_alpha_q_im = cplx.alpha_q_im[g][b - 1] as i32;
                                } else {
                                    last_alpha_q_re = 0;
                                    last_alpha_q_im = 0;
                                }
                            } /* if (delta_code_time == 1) */

                            if ms_get(&jsd.ms_used, band)
                                & (1u8 as i32).wrapping_shl(group as u32) as u8
                                != 0
                            {
                                let mut dpcm_alpha_re = cblock_decode_huffman_word(bs, hcb);
                                dpcm_alpha_re -= 60;
                                dpcm_alpha_re *= -1;

                                cplx.alpha_q_re[g][b] = (dpcm_alpha_re + last_alpha_q_re) as i16;

                                if cplx.complex_coef != 0 {
                                    let mut dpcm_alpha_im = cblock_decode_huffman_word(bs, hcb);
                                    dpcm_alpha_im -= 60;
                                    dpcm_alpha_im *= -1;

                                    cplx.alpha_q_im[g][b] =
                                        (dpcm_alpha_im + last_alpha_q_im) as i16;
                                } else {
                                    cplx.alpha_q_im[g][b] = 0;
                                }
                            } else {
                                cplx.alpha_q_re[g][b] = 0;
                                cplx.alpha_q_im[g][b] = 0;
                            } /* if (pJointStereoData->MsUsed[band] & ((UCHAR)1 << group)) */

                            if (band + 1) < scale_factor_bands_transmitted && b + 1 < bmax as usize
                            {
                                /* <= this should be the correct way (cp.
                                ISO_IEC_FDIS_23003-0(E) */
                                /*    7.7.2.3.2 Decoding of prediction coefficients) */
                                cplx.alpha_q_re[g][b + 1] = cplx.alpha_q_re[g][b];
                                cplx.alpha_q_im[g][b + 1] = cplx.alpha_q_im[g][b];
                            } /* if ((band+1)<scaleFactorBandsTotal) */

                            pjs.alpha_q_re_prev[g][b] = cplx.alpha_q_re[g][b];
                            pjs.alpha_q_im_prev[g][b] = cplx.alpha_q_im[g][b];
                            band += SFB_PER_PRED_BAND;
                        }

                        for band in
                            scale_factor_bands_transmitted.max(0)..max_sfb_ste_clear.min(bmax)
                        {
                            let b = band as usize;
                            cplx.alpha_q_re[g][b] = 0;
                            cplx.alpha_q_im[g][b] = 0;
                            pjs.alpha_q_re_prev[g][b] = 0;
                            pjs.alpha_q_im_prev[g][b] = 0;
                        }
                    }
                }
            }
        } else {
            for group in 0..window_groups.clamp(0, JointStereoMaximumGroups) {
                for band in 0..max_sfb_ste_clear.clamp(0, JointStereoMaximumBands) {
                    pjs.alpha_q_re_prev[group as usize][band as usize] = 0;
                    pjs.alpha_q_im_prev[group as usize][band as usize] = 0;
                }
            }
        }

        pjs.win_groups_prev = window_groups as u8;
    }

    0
}

/// `CJointStereo_filterAndAdd(in, len, windowLen, coeff, out, isCurrent)` (static in C): the
/// MDST estimation filter. `input` and `out` start at `in` and `out` of C; `len` lines are
/// processed. Indexes outside the slices (`len` below 7) read 0 and are not written.
pub fn cjoint_stereo_filter_and_add(
    input: &[FixpDbl],
    len: i32,
    _window_len: i32,
    coeff: &[FixpFilt],
    out: &mut [FixpDbl],
    is_current: u8, /* output values with even index get a positve addon (=1) or a negative
                    addon (=0) */
) {
    // The function-local index tables of the C++ function, dumped with (the declarations of the
    // C++ source, renamed and made `static const`):
    //   scripts/dump_table.py libAACdec/src/stereo.cpp indices_1_fnlocal INT --prelude 'static const int indices_1_fnlocal[] = {2, 1, 0, 1, 2, 3};'
    //   scripts/dump_table.py libAACdec/src/stereo.cpp indices_2_fnlocal INT --prelude 'static const int indices_2_fnlocal[] = {1, 0, 0, 2, 3, 4};'
    //   scripts/dump_table.py libAACdec/src/stereo.cpp indices_3_fnlocal INT --prelude 'static const int indices_3_fnlocal[] = {0, 0, 1, 3, 4, 5};'
    //   scripts/dump_table.py libAACdec/src/stereo.cpp subtr_1_fnlocal INT --prelude 'static const int subtr_1_fnlocal[] = {6, 5, 4, 2, 1, 1};'
    //   scripts/dump_table.py libAACdec/src/stereo.cpp subtr_2_fnlocal INT --prelude 'static const int subtr_2_fnlocal[] = {5, 4, 3, 1, 1, 2};'
    //   scripts/dump_table.py libAACdec/src/stereo.cpp subtr_3_fnlocal INT --prelude 'static const int subtr_3_fnlocal[] = {4, 3, 2, 1, 2, 3};'
    // The cref test `filter_and_add` compares them through this function.
    const INDICES_1: [i32; 6] = [2, 1, 0, 1, 2, 3];
    const INDICES_2: [i32; 6] = [1, 0, 0, 2, 3, 4];
    const INDICES_3: [i32; 6] = [0, 0, 1, 3, 4, 5];

    const SUBTR_1: [i32; 6] = [6, 5, 4, 2, 1, 1];
    const SUBTR_2: [i32; 6] = [5, 4, 3, 1, 1, 2];
    const SUBTR_3: [i32; 6] = [4, 3, 2, 1, 2, 3];

    let inp = |i: i32| -> FixpDbl { input.get(i as usize).copied().unwrap_or(0) };
    let c = |i: usize| -> FixpFilt { coeff.get(i).copied().unwrap_or(0) };
    // out[o] -= / += v
    fn sub(out: &mut [FixpDbl], o: i32, v: FixpDbl) {
        if let Some(x) = out.get_mut(o as usize) {
            *x = x.wrapping_sub(v);
        }
    }
    fn add(out: &mut [FixpDbl], o: i32, v: FixpDbl) {
        if let Some(x) = out.get_mut(o as usize) {
            *x = x.wrapping_add(v);
        }
    }

    if is_current == 1 {
        /* exploit the symmetry of the table: coeff[6] = - coeff[0],
                                              coeff[5] = - coeff[1],
                                              coeff[4] = - coeff[2],
                                              coeff[3] = 0
        */

        for i in 0..3 {
            sub(out, 0, f_mult_div2(c(i), inp(INDICES_1[i])) >> SR_FNA_OUT);
            add(out, 0, f_mult_div2(c(i), inp(INDICES_1[5 - i])) >> SR_FNA_OUT);
        }

        for i in 0..3 {
            sub(out, 1, f_mult_div2(c(i), inp(INDICES_2[i])) >> SR_FNA_OUT);
            add(out, 1, f_mult_div2(c(i), inp(INDICES_2[5 - i])) >> SR_FNA_OUT);
        }

        for i in 0..3 {
            sub(out, 2, f_mult_div2(c(i), inp(INDICES_3[i])) >> SR_FNA_OUT);
            add(out, 2, f_mult_div2(c(i), inp(INDICES_3[5 - i])) >> SR_FNA_OUT);
        }

        for j in 3..(len - 3) {
            for i in 0..3 {
                sub(out, j, f_mult_div2(c(i), inp(j - 3 + i as i32)) >> SR_FNA_OUT);
                add(out, j, f_mult_div2(c(i), inp(j + 3 - i as i32)) >> SR_FNA_OUT);
            }
        }

        for i in 0..3 {
            sub(out, len - 3, f_mult_div2(c(i), inp(len - SUBTR_1[i])) >> SR_FNA_OUT);
            add(out, len - 3, f_mult_div2(c(i), inp(len - SUBTR_1[5 - i])) >> SR_FNA_OUT);
        }

        for i in 0..3 {
            sub(out, len - 2, f_mult_div2(c(i), inp(len - SUBTR_2[i])) >> SR_FNA_OUT);
            add(out, len - 2, f_mult_div2(c(i), inp(len - SUBTR_2[5 - i])) >> SR_FNA_OUT);
        }

        for i in 0..3 {
            sub(out, len - 1, f_mult_div2(c(i), inp(len - SUBTR_3[i])) >> SR_FNA_OUT);
            add(out, len - 1, f_mult_div2(c(i), inp(len - SUBTR_3[5 - i])) >> SR_FNA_OUT);
        }
    } else {
        /* exploit the symmetry of the table: coeff[6] = coeff[0],
                                              coeff[5] = coeff[1],
                                              coeff[4] = coeff[2]
        */

        for i in 0..3 {
            sub(out, 0, f_mult_div2(c(i), inp(INDICES_1[i]) >> SR_FNA_OUT));
            sub(out, 0, f_mult_div2(c(i), inp(INDICES_1[5 - i]) >> SR_FNA_OUT));
        }
        sub(out, 0, f_mult_div2(c(3), inp(0) >> SR_FNA_OUT));

        for i in 0..3 {
            add(out, 1, f_mult_div2(c(i), inp(INDICES_2[i]) >> SR_FNA_OUT));
            add(out, 1, f_mult_div2(c(i), inp(INDICES_2[5 - i]) >> SR_FNA_OUT));
        }
        add(out, 1, f_mult_div2(c(3), inp(1) >> SR_FNA_OUT));

        for i in 0..3 {
            sub(out, 2, f_mult_div2(c(i), inp(INDICES_3[i]) >> SR_FNA_OUT));
            sub(out, 2, f_mult_div2(c(i), inp(INDICES_3[5 - i]) >> SR_FNA_OUT));
        }
        sub(out, 2, f_mult_div2(c(3), inp(2) >> SR_FNA_OUT));

        let mut j = 3;
        while j < (len - 4) {
            for i in 0..3 {
                add(out, j, f_mult_div2(c(i), inp(j - 3 + i as i32) >> SR_FNA_OUT));
                add(out, j, f_mult_div2(c(i), inp(j + 3 - i as i32) >> SR_FNA_OUT));
            }
            add(out, j, f_mult_div2(c(3), inp(j) >> SR_FNA_OUT));

            j += 1;

            for i in 0..3 {
                sub(out, j, f_mult_div2(c(i), inp(j - 3 + i as i32) >> SR_FNA_OUT));
                sub(out, j, f_mult_div2(c(i), inp(j + 3 - i as i32) >> SR_FNA_OUT));
            }
            sub(out, j, f_mult_div2(c(3), inp(j) >> SR_FNA_OUT));
            j += 1;
        }

        for i in 0..3 {
            add(out, len - 3, f_mult_div2(c(i), inp(len - SUBTR_1[i]) >> SR_FNA_OUT));
            add(out, len - 3, f_mult_div2(c(i), inp(len - SUBTR_1[5 - i]) >> SR_FNA_OUT));
        }
        add(out, len - 3, f_mult_div2(c(3), inp(len - 3) >> SR_FNA_OUT));

        for i in 0..3 {
            sub(out, len - 2, f_mult_div2(c(i), inp(len - SUBTR_2[i]) >> SR_FNA_OUT));
            sub(out, len - 2, f_mult_div2(c(i), inp(len - SUBTR_2[5 - i]) >> SR_FNA_OUT));
        }
        sub(out, len - 2, f_mult_div2(c(3), inp(len - 2) >> SR_FNA_OUT));

        for i in 0..3 {
            add(out, len - 1, f_mult_div2(c(i), inp(len - SUBTR_3[i]) >> SR_FNA_OUT));
            add(out, len - 1, f_mult_div2(c(i), inp(len - SUBTR_3[5 - i]) >> SR_FNA_OUT));
        }
        add(out, len - 1, f_mult_div2(c(3), inp(len - 1) >> SR_FNA_OUT));
    }
}

/// `CJointStereo_GenerateMSOutput(pSpecLCurrBand, pSpecRCurrBand, leftScale, rightScale,
/// nSfbBands)` (static inline in C). `n_sfb_bands` is a multiple of 4 in the library (every band
/// width), for which C counts down an unsigned index in steps of 4. The port processes the
/// complete groups of 4 lines from the end of the band, like C, and stops when fewer than 4
/// lines are left.
///
/// Optimized: the lines are independent of each other, so the lines that the groups of 4 cover
/// (`n % 4..n`) are processed in one forward loop on slices (vectorized);
/// [`cjoint_stereo_generate_ms_output_straight`] is the port.
pub fn cjoint_stereo_generate_ms_output(
    p_spec_l_curr_band: &mut [FixpDbl],
    p_spec_r_curr_band: &mut [FixpDbl],
    left_scale: u32,
    right_scale: u32,
    n_sfb_bands: u32,
) {
    let n = n_sfb_bands as usize;
    if n > p_spec_l_curr_band.len() || n > p_spec_r_curr_band.len() {
        return;
    }
    let first = n % 4;
    for (l, r) in p_spec_l_curr_band[first..n].iter_mut().zip(&mut p_spec_r_curr_band[first..n]) {
        /* MS output generation */
        let lv = l.wrapping_shr(left_scale);
        let rv = r.wrapping_shr(right_scale);
        *l = lv.wrapping_add(rv);
        *r = lv.wrapping_sub(rv);
    }
}

/// `CJointStereo_GenerateMSOutput()` as ported (see [`cjoint_stereo_generate_ms_output`]).
pub fn cjoint_stereo_generate_ms_output_straight(
    p_spec_l_curr_band: &mut [FixpDbl],
    p_spec_r_curr_band: &mut [FixpDbl],
    left_scale: u32,
    right_scale: u32,
    n_sfb_bands: u32,
) {
    let mut i = (n_sfb_bands as usize).min(p_spec_l_curr_band.len()).min(p_spec_r_curr_band.len());
    if (n_sfb_bands as usize) != i {
        return;
    }
    while i >= 4 {
        for k in (i - 4)..i {
            /* MS output generation */
            let l = p_spec_l_curr_band[k].wrapping_shr(left_scale);
            let r = p_spec_r_curr_band[k].wrapping_shr(right_scale);
            p_spec_l_curr_band[k] = l.wrapping_add(r);
            p_spec_r_curr_band[k] = l.wrapping_sub(r);
        }
        i -= 4;
    }
}

/// `x >>= amount` for a shift amount that the library keeps within 0..=31 (lsl/asr with a
/// register amount in both builds, so the amount is taken modulo 32).
#[inline]
fn shr(x: FixpDbl, amount: i32) -> FixpDbl {
    x.wrapping_shr(amount as u32)
}

/// `CJointStereo_ApplyMS(pAacDecoderChannelInfo[2], pAacDecoderStaticChannelInfo[2], spectrumL,
/// spectrumR, SFBleftScale, SFBrightScale, specScaleL, specScaleR, pScaleFactorBandOffsets,
/// pWindowGroupLength, windowGroups, max_sfb_ste_outside, scaleFactorBandsTransmittedL,
/// scaleFactorBandsTransmittedR, store_dmx_re_prev, store_dmx_re_prev_e, mainband_flag)`: apply
/// M/S stereo or complex stereo prediction.
///
/// The parameters that always point to fields of the channels or of `pComData` are dropped
/// (see the [module documentation](self)): `spectrumL/R` are `spec[0..]` and `spec[1024..]`,
/// `SFBleftScale/SFBrightScale` the `aSfbScale` of the channels, `specScaleL/R` their
/// `specScale`, `store_dmx_re_prev(_e)` the fields of `com.joint_stereo_data`.
///
/// Further memory: `spec` (the spectrum of the element), `com` (`pComData`: `jointStereoData`),
/// `com_static` (`pComStaticData->cplxPredictionData`), `cpe` (`pCpeStaticData` of the first
/// static channel info; `None` for `NULL`), `scratch` (`jointStereoPersistentData.scratchBuffer`,
/// at least 2048 values: `dmx_re` and `dmx_re_prev`). The spectra and scale factors of the
/// previous frame (`spectralCoeffs[2]`, `specScale[2]`) are the concealment buffers of the two
/// static channel infos.
///
/// With complex prediction signalled but `cpe` or `cplxPredictionData` missing (no defined
/// result in the reference), the port uses zero-initialized structures.
pub fn cjoint_stereo_apply_ms(
    p_aac_decoder_channel_info: &mut [&mut CAacDecoderChannelInfo],
    p_aac_decoder_static_channel_info: &mut [&mut CAacDecoderStaticChannelInfo],
    p_scale_factor_band_offsets: &[i16],
    p_window_group_length: &[u8],
    window_groups: i32,
    max_sfb_ste_outside: i32,
    scale_factor_bands_transmitted_l: i32,
    scale_factor_bands_transmitted_r: i32,
    mainband_flag: i32,
    spec: &mut [FixpDbl],
    com: &mut CAacDecoderCommonData,
    com_static: &mut CAacDecoderCommonStaticData,
    cpe: Option<&mut CpePersistentData>,
    scratch: &mut [FixpDbl],
) {
    let offsets = p_scale_factor_band_offsets;
    let off = |b: i32| -> i32 { offsets.get(b as usize).copied().unwrap_or(0) as i32 };
    let wgl =
        |g: i32| -> i32 { p_window_group_length.get(g as usize).copied().unwrap_or(0) as i32 };

    let max_sfb_ste = scale_factor_bands_transmitted_l.max(scale_factor_bands_transmitted_r);
    let min_sfb_ste = scale_factor_bands_transmitted_l.min(scale_factor_bands_transmitted_r);
    let scale_factor_bands_transmitted = min_sfb_ste;

    let (spec_l, spec_r) = {
        let n = spec.len();
        let (a, b) = spec.split_at_mut(1024.min(n));
        (a, b)
    };
    let (pl, pr) = {
        let (a, b) = p_aac_decoder_channel_info.split_at_mut(1);
        (&mut *a[0], &mut *b[0])
    };
    let jsd = &mut com.joint_stereo_data;

    if jsd.cplx_pred_flag != 0 {
        let dummy_cplx;
        let cplx: &CCplxPredictionData = match com_static.cplx_prediction_data.as_deref() {
            Some(c) => c,
            None => {
                dummy_cplx = CCplxPredictionData::default();
                &dummy_cplx
            }
        };
        let mut dummy_cpe;
        let cpe: &mut CpePersistentData = match cpe {
            Some(c) => c,
            None => {
                dummy_cpe = CpePersistentData::default();
                &mut dummy_cpe
            }
        };
        let pjs = &mut cpe.joint_stereo_persistent_data;
        let (psl, psr) = {
            let (a, b) = p_aac_decoder_static_channel_info.split_at_mut(1);
            (&*a[0], &*b[0])
        };
        let static_spectral_coeffs_l = &psl.concealment_info.spectral_coefficient;
        let static_spectral_coeffs_r = &psr.concealment_info.spectral_coefficient;
        let static_spec_scale_l = &psl.concealment_info.spec_scale;
        let static_spec_scale_r = &psr.concealment_info.spec_scale;

        let (dmx_re, dmx_re_prev) = {
            let n = scratch.len();
            scratch.split_at_mut(1024.min(n))
        };
        let dmxr = |dmx_re: &[FixpDbl], i: i32| -> FixpDbl {
            dmx_re.get(i as usize).copied().unwrap_or(0)
        };

        /* When MS is applied over the main band this value gets computed. Otherwise (for the
         * tiles) it uses the assigned value */
        let mut dmx_re_prev_e: i16 = jsd.store_dmx_re_prev_e;

        let previous_shape = pjs.win_shape_prev as i32;
        let current_shape = pl.ics_info.window_shape as i32;

        /* complex stereo prediction */

        /* 0. preparations */

        /* 0.0. get scratch buffer for downmix MDST */
        // C_AALLOC_SCRATCH_START(dmx_im, FIXP_DBL, 1024): every part that is read is cleared
        // first (FDKmemclear per window).
        let mut dmx_im: [FixpDbl; 1024] = [0; 1024];

        /* 0.1. window lengths */

        /* get length of short window for current configuration */
        let mut window_len: i32 = pl.granule_length; /* framelength 768 => 96, framelength 1024 => 128 */

        /* if this is no short-block set length for long-block */
        if pl.ics_info.window_sequence != BLOCK_SHORT {
            window_len *= 8;
        }

        let coef_prev = |i: i32| -> &'static [FixpFilt] {
            MDST_FILT_COEF_PREV.get(i as usize).map(|r| &r[..]).unwrap_or(&[])
        };
        let coef_curr = |i: i32| -> &'static [FixpFilt] {
            MDST_FILT_COEF_CURR.get(i as usize).map(|r| &r[..]).unwrap_or(&[])
        };

        /* 0.2. set pointer to filter-coefficients for MDST excitation including previous frame
         * portions */
        /*      cp. ISO/IEC FDIS 23003-3:2011(E) table 125 */

        /* set pointer to default-position */
        let mut p_coeff_prev = coef_prev(previous_shape);

        if cplx.complex_coef == 1 {
            match pl.ics_info.window_sequence {
                /* current window sequence */
                BLOCK_SHORT | BLOCK_LONG => {
                    p_coeff_prev = coef_prev(previous_shape);
                }
                BLOCK_START => {
                    if (pjs.win_seq_prev as i32 == BLOCK_SHORT)
                        || (pjs.win_seq_prev as i32 == BLOCK_START)
                    {
                        /* a stop-start-sequence can only follow on an eight-short-sequence or
                         * a start-sequence */
                        p_coeff_prev = coef_prev(2 + previous_shape);
                    } else {
                        p_coeff_prev = coef_prev(previous_shape);
                    }
                }
                BLOCK_STOP => {
                    p_coeff_prev = coef_prev(2 + previous_shape);
                }
                _ => {
                    p_coeff_prev = coef_prev(previous_shape);
                }
            }
        }

        /* 0.3. set pointer to filter-coefficients for MDST excitation */

        /* define offset of pointer to filter-coefficients for MDST exitation employing only the
         * current frame */
        let coeff_pointer_offset =
            if (previous_shape == SHAPE_SINE) && (current_shape == SHAPE_SINE) {
                0
            } else if (previous_shape == SHAPE_SINE) && (current_shape == SHAPE_KBD) {
                2
            } else if (previous_shape == SHAPE_KBD) && (current_shape == SHAPE_KBD) {
                1
            } else {
                /* if ( (previousShape == SHAPE_KBD) && (currentShape == SHAPE_SINE) ) */
                3
            };

        /* set pointer to filter-coefficient table cp. ISO/IEC FDIS 23003-3:2011(E) table 124 */
        let mut p_coeff = match pl.ics_info.window_sequence {
            /* current window sequence */
            BLOCK_SHORT | BLOCK_LONG => coef_curr(coeff_pointer_offset),
            BLOCK_START => {
                if (pjs.win_seq_prev as i32 == BLOCK_SHORT)
                    || (pjs.win_seq_prev as i32 == BLOCK_START)
                {
                    /* a stop-start-sequence can only follow on an eight-short-sequence or a
                     * start-sequence */
                    coef_curr(12 + coeff_pointer_offset)
                } else {
                    coef_curr(4 + coeff_pointer_offset)
                }
            }
            BLOCK_STOP => coef_curr(8 + coeff_pointer_offset),
            _ => coef_curr(coeff_pointer_offset),
        };

        let sfb_l = |w: i32, b: i32, pl: &CAacDecoderChannelInfo| -> i32 {
            pl.p_dyn_data.a_sfb_scale.get((w * 16 + b) as usize).copied().unwrap_or(0) as i32
        };

        /* 0.4. find maximum common (l/r) band-scaling-factor for whole sequence (all windows) */
        let mut frame_max_scale: i32 = 0;
        let mut window: i32 = 0;
        for group in 0..window_groups {
            for _groupwin in 0..wgl(group) {
                let mut window_max_scale: i32 = 0;

                /* find maximum scaling factor of all bands in this window */
                let mut band = 0;
                while band < min_sfb_ste {
                    let l_scale = sfb_l(window, band, pl);
                    let r_scale = sfb_l(window, band, pr);
                    let common_scale = if l_scale > r_scale { l_scale } else { r_scale };
                    window_max_scale = if window_max_scale < common_scale {
                        common_scale
                    } else {
                        window_max_scale
                    };
                    band += 1;
                }
                if scale_factor_bands_transmitted_l > min_sfb_ste {
                    /* i.e. scaleFactorBandsTransmittedL == max_sfb_ste */
                    while band < max_sfb_ste {
                        let l_scale = sfb_l(window, band, pl);
                        window_max_scale =
                            if window_max_scale < l_scale { l_scale } else { window_max_scale };
                        band += 1;
                    }
                } else {
                    if scale_factor_bands_transmitted_r > min_sfb_ste {
                        /* i.e. scaleFactorBandsTransmittedR == max_sfb_ste */
                        while band < max_sfb_ste {
                            let r_scale = sfb_l(window, band, pr);
                            window_max_scale =
                                if window_max_scale < r_scale { r_scale } else { window_max_scale };
                            band += 1;
                        }
                    }
                }

                /* find maximum common SF of all windows */
                frame_max_scale = if frame_max_scale < window_max_scale {
                    window_max_scale
                } else {
                    frame_max_scale
                };
                window += 1;
            }
        }

        /* add some headroom for overflow protection during filter and add operation */
        frame_max_scale += 2;

        /* process on window-basis (i.e. iterate over all groups and corresponding windows) */
        let mut window: i32 = 0;
        for group in 0..window_groups {
            let group_mask: u8 = (1i32 << (group & 31)) as u8;

            for _groupwin in 0..wgl(group) {
                if window >= 8 {
                    // An inconsistent window grouping with more than 8 windows.
                    break;
                }
                let wu = window as usize;
                let wl = window_len.max(0) as usize;
                /* initialize the MDST with zeros */
                {
                    let b = (wl * wu).min(1024);
                    let e = (b + wl).min(1024);
                    dmx_im[b..e].fill(0);
                }

                /* 1. calculate the previous downmix MDCT. We do this once just for the Main
                 * band. */
                if cplx.complex_coef == 1 {
                    if (cplx.use_prev_frame == 1) && (mainband_flag != 0) {
                        /* if this is a long-block or the first window of a short-block calculate
                        the downmix MDCT of the previous frame. use_prev_frame is assumed not
                        to change during a frame! */

                        /* first determine shiftfactors to scale left and right channel */
                        if (pl.ics_info.window_sequence != BLOCK_SHORT) || (window == 0) {
                            let mut index_offset: usize = 0;
                            let mut sr_left_chan: i32 = 0;
                            let mut sr_right_chan: i32 = 0;
                            if pl.ics_info.window_sequence == BLOCK_SHORT {
                                /* use the last window of the previous frame for MDCT calculation
                                 * if this is a short-block. */
                                index_offset = wl * 7;
                                if static_spec_scale_l[7] > static_spec_scale_r[7] {
                                    sr_right_chan = static_spec_scale_l[7] as i32
                                        - static_spec_scale_r[7] as i32;
                                    dmx_re_prev_e = static_spec_scale_l[7];
                                } else {
                                    sr_left_chan = static_spec_scale_r[7] as i32
                                        - static_spec_scale_l[7] as i32;
                                    dmx_re_prev_e = static_spec_scale_r[7];
                                }
                            } else {
                                if static_spec_scale_l[0] > static_spec_scale_r[0] {
                                    sr_right_chan = static_spec_scale_l[0] as i32
                                        - static_spec_scale_r[0] as i32;
                                    dmx_re_prev_e = static_spec_scale_l[0];
                                } else {
                                    sr_left_chan = static_spec_scale_r[0] as i32
                                        - static_spec_scale_l[0] as i32;
                                    dmx_re_prev_e = static_spec_scale_r[0];
                                }
                            }

                            let n = wl.min(dmx_re_prev.len());
                            /* now scale channels and determine downmix MDCT of previous frame */
                            if pjs.clear_spectral_coeffs == 1 {
                                dmx_re_prev[..n].fill(0);
                                dmx_re_prev_e = 0;
                            } else {
                                let sl = (DFRACT_BITS - 1).min(sr_left_chan + 1);
                                let sr = (DFRACT_BITS - 1).min(sr_right_chan + 1);
                                let sc = |c: &[FixpDbl], i: usize| -> FixpDbl {
                                    c.get(index_offset + i).copied().unwrap_or(0)
                                };
                                if cplx.pred_dir == 0 {
                                    for i in 0..n {
                                        dmx_re_prev[i] = shr(sc(static_spectral_coeffs_l, i), sl)
                                            .wrapping_add(shr(sc(static_spectral_coeffs_r, i), sr));
                                    }
                                } else {
                                    for i in 0..n {
                                        dmx_re_prev[i] = shr(sc(static_spectral_coeffs_l, i), sl)
                                            .wrapping_sub(shr(sc(static_spectral_coeffs_r, i), sr));
                                    }
                                }
                            }

                            /* In case that we use INF we have to preserve the state of the
                            "dmx_re_prev" (original or computed). This is necessary because we have
                            to apply MS over the separate IGF tiles. */
                            let n2 = n.min(jsd.store_dmx_re_prev.len());
                            jsd.store_dmx_re_prev[..n2].copy_from_slice(&dmx_re_prev[..n2]);

                            /* Particular exponent of the computed/original "dmx_re_prev" must be
                             * kept for the tile MS calculations if necessary.*/
                            jsd.store_dmx_re_prev_e = dmx_re_prev_e;
                        } /* if ( (pAacDecoderChannelInfo[L]->icsInfo.WindowSequence !=
                          BLOCK_SHORT) || (window == 0) ) */
                    } /* if ( pJointStereoData->use_prev_frame == 1 ) */
                } /* if ( pJointStereoData->complex_coef == 1 ) */

                /* 2. calculate downmix MDCT of current frame */

                pl.spec_scale[wu] = frame_max_scale as i16;
                pr.spec_scale[wu] = frame_max_scale as i16;

                /* adapt scaling-factors to previous frame */
                if cplx.use_prev_frame == 1 {
                    if window == 0 {
                        let n = wl.min(dmx_re_prev.len());
                        if (dmx_re_prev_e as i32) < frame_max_scale {
                            let s =
                                -((DFRACT_BITS - 1).min(frame_max_scale - dmx_re_prev_e as i32));
                            if mainband_flag == 0 {
                                let n2 = n.min(jsd.store_dmx_re_prev.len());
                                scale_values_to(
                                    &mut dmx_re_prev[..n2],
                                    &jsd.store_dmx_re_prev[..n2],
                                    s,
                                );
                            } else {
                                scale_values(&mut dmx_re_prev[..n], s);
                            }
                        } else {
                            if mainband_flag == 0 {
                                let n2 = n.min(jsd.store_dmx_re_prev.len());
                                dmx_re_prev[..n2].copy_from_slice(&jsd.store_dmx_re_prev[..n2]);
                            }
                            pl.spec_scale[0] = dmx_re_prev_e;
                            pr.spec_scale[0] = dmx_re_prev_e;
                        }
                    } else {
                        /* window != 0 */
                        fdk_assert!(pl.ics_info.window_sequence == BLOCK_SHORT);
                        if (pl.spec_scale[wu - 1] as i32) < frame_max_scale {
                            let b = (wl * (wu - 1)).min(dmx_re.len());
                            let e = (b + wl).min(dmx_re.len());
                            scale_values(
                                &mut dmx_re[b..e],
                                -((DFRACT_BITS - 1)
                                    .min(frame_max_scale - pl.spec_scale[wu - 1] as i32)),
                            );
                        } else {
                            pl.spec_scale[wu] = pl.spec_scale[wu - 1];
                            pr.spec_scale[wu] = pr.spec_scale[wu - 1];
                        }
                    }
                } /* if ( pJointStereoData->use_prev_frame == 1 ) */

                /* scaling factors of both channels ought to be equal now */
                fdk_assert!(pl.spec_scale[wu] == pr.spec_scale[wu]);

                /* rescale signal and calculate downmix MDCT */
                let base = wl * wu;
                for band in 0..max_sfb_ste {
                    /* first adapt scaling of current band to scaling of current window => shift
                     * signal right */
                    let si = (window * 16 + band) as usize;
                    let l_scale0 = pl.p_dyn_data.a_sfb_scale.get(si).copied().unwrap_or(0) as i32;
                    let r_scale0 = pr.p_dyn_data.a_sfb_scale.get(si).copied().unwrap_or(0) as i32;

                    let l_scale = (DFRACT_BITS - 1).min(pl.spec_scale[wu] as i32 - l_scale0);
                    let r_scale = (DFRACT_BITS - 1).min(pl.spec_scale[wu] as i32 - r_scale0); /* L or R doesn't matter, specScales are equal at this point */

                    /* Write back to sfb scale to cover the case when max_sfb_ste < max_sfb */
                    if si < 128 {
                        pl.p_dyn_data.a_sfb_scale[si] = pl.spec_scale[wu];
                        pr.p_dyn_data.a_sfb_scale[si] = pl.spec_scale[wu];
                    }

                    for i in off(band)..off(band + 1) {
                        let k = base + i as usize;
                        if let Some(v) = spec_l.get_mut(k) {
                            *v = shr(*v, l_scale);
                        }
                        if let Some(v) = spec_r.get_mut(k) {
                            *v = shr(*v, r_scale);
                        }
                    }

                    /* now calculate downmix MDCT */
                    let gl = |k: usize| spec_l.get(k).copied().unwrap_or(0);
                    let gr = |k: usize| spec_r.get(k).copied().unwrap_or(0);
                    if jsd.ms_used.get(band as usize).copied().unwrap_or(0) & group_mask != 0 {
                        for i in off(band)..off(band + 1) {
                            let k = base + i as usize;
                            if let Some(d) = dmx_re.get_mut(k) {
                                *d = gl(k);
                            }
                        }
                    } else {
                        if cplx.pred_dir == 0 {
                            for i in off(band)..off(band + 1) {
                                let k = base + i as usize;
                                if let Some(d) = dmx_re.get_mut(k) {
                                    *d = gl(k).wrapping_add(gr(k)) >> 1;
                                }
                            }
                        } else {
                            for i in off(band)..off(band + 1) {
                                let k = base + i as usize;
                                if let Some(d) = dmx_re.get_mut(k) {
                                    *d = gl(k).wrapping_sub(gr(k)) >> 1;
                                }
                            }
                        }
                    }
                } /* for ( band=0; band<max_sfb_ste; band++ ) */
                /* Clean until the end */
                for i in off(max_sfb_ste_outside)..window_len {
                    if let Some(d) = dmx_re.get_mut(base + i as usize) {
                        *d = 0;
                    }
                }

                /* 3. calculate MDST-portion corresponding to the current frame. */
                if cplx.complex_coef == 1 {
                    {
                        /* 3.1 move pointer in filter-coefficient table in case of short window
                         * sequence */
                        /*     (other coefficients are utilized for the last 7 short windows) */
                        if (pl.ics_info.window_sequence == BLOCK_SHORT) && (window != 0) {
                            p_coeff = coef_curr(current_shape);
                            p_coeff_prev = coef_prev(current_shape);
                        }

                        /* The length of the filter processing must be extended because of filter
                         * boundary problems */
                        let extended_band = (off(max_sfb_ste_outside) + 7).min(window_len);

                        /* 3.2. estimate downmix MDST from current frame downmix MDCT */
                        let im_off = base.min(1024);
                        if (pl.ics_info.window_sequence == BLOCK_SHORT) && (window != 0) {
                            let re_off = base.min(dmx_re.len());
                            cjoint_stereo_filter_and_add(
                                &dmx_re[re_off..],
                                extended_band,
                                window_len,
                                p_coeff,
                                &mut dmx_im[im_off..],
                                1,
                            );

                            let re_off = (wl * (wu - 1)).min(dmx_re.len());
                            cjoint_stereo_filter_and_add(
                                &dmx_re[re_off..],
                                extended_band,
                                window_len,
                                p_coeff_prev,
                                &mut dmx_im[im_off..],
                                0,
                            );
                        } else {
                            cjoint_stereo_filter_and_add(
                                dmx_re,
                                extended_band,
                                window_len,
                                p_coeff,
                                &mut dmx_im[..],
                                1,
                            );

                            if cplx.use_prev_frame == 1 {
                                cjoint_stereo_filter_and_add(
                                    dmx_re_prev,
                                    extended_band,
                                    window_len,
                                    p_coeff_prev,
                                    &mut dmx_im[im_off..],
                                    0,
                                );
                            }
                        }
                    } /* if(pAacDecoderChannelInfo[L]->transform_splitting_active) */
                } /* if ( pJointStereoData->complex_coef == 1 ) */

                /* 4. upmix process */
                let pred_dir: i32 = if cplx.pred_dir != 0 { -1 } else { 1 };
                /* 0.1 in Q-3.34 */
                let point_one: FixpDbl = 0x66666666; /* 0.8 */
                /* Shift value for the downmix */
                let shift_dmx: i32 = SF_FNA_COEFFS + 1;

                let gi = (group as usize).min(JointStereoMaximumGroups as usize - 1);
                for band in 0..max_sfb_ste_outside {
                    if jsd.ms_used.get(band as usize).copied().unwrap_or(0) & group_mask != 0 {
                        let bi = (band as usize).min(JointStereoMaximumBands as usize - 1);
                        let temp_re: FixpSgl = cplx.alpha_q_re[gi][bi];
                        let temp_im: FixpSgl = cplx.alpha_q_im[gi][bi];

                        /* Find the minimum common headroom for alpha_re and alpha_im */
                        let mut alpha_re_headroom = count_leading_bits(temp_re as i32) - 16;
                        if temp_re == 0 {
                            alpha_re_headroom = 15;
                        }
                        let mut alpha_im_headroom = count_leading_bits(temp_im as i32) - 16;
                        if temp_im == 0 {
                            alpha_im_headroom = 15;
                        }
                        let val = alpha_re_headroom.min(alpha_im_headroom);

                        /* Multiply alpha by 0.1 with maximum precision */
                        fdk_assert!(val >= 0);
                        let alpha_re_tmp: FixpDbl = f_mult(
                            ((temp_re as i32).wrapping_shl(val as u32)) as FixpSgl,
                            point_one,
                        );
                        let alpha_im_tmp: FixpDbl = f_mult(
                            ((temp_im as i32).wrapping_shl(val as u32)) as FixpSgl,
                            point_one,
                        );

                        /* Calculate alpha exponent */
                        /* (Q-3.34 * Q15.0) shifted left by "val" */
                        let alpha_re_exp = -3 + 15 - val;

                        let help3_shift = alpha_re_exp + 1;

                        for i in off(band)..off(band + 1) {
                            let k = base + i as usize;
                            /* Calculating helper term:
                                  side = specR[i] - alpha_re[i] * dmx_re[i] - alpha_im[i] *
                              dmx_im[i];

                              Here "dmx_re" may be the same as "specL" or alternatively keep
                              the downmix. "dmx_re" and "specL" are two different pointers
                              pointing to separate arrays, which may or may not contain the
                              same data (with different scaling).

                              specL[i] =   + (specL[i] + side);
                              specR[i] = -/+ (specL[i] - side);
                            */
                            let dre = dmxr(dmx_re, k as i32);
                            let dim = dmx_im.get(k).copied().unwrap_or(0);
                            let mut side = f_mult_add_div2(
                                f_mult_div2(alpha_re_tmp, dre),
                                alpha_im_tmp,
                                dim.wrapping_shl(shift_dmx as u32),
                            );
                            let cl = spec_l.get(k).copied().unwrap_or(0);
                            let cr = spec_r.get(k).copied().unwrap_or(0);
                            side = (cr >> 2).wrapping_sub(saturate_shift(
                                side,
                                -(help3_shift - 2),
                                DFRACT_BITS - 2,
                            ));

                            let left = (cl >> 2).wrapping_add(side);
                            let mut right = (cl >> 2).wrapping_sub(side);
                            right = right.wrapping_mul(pred_dir);

                            if let Some(v) = spec_l.get_mut(k) {
                                *v = saturate_left_shift_alt(left, 2, DFRACT_BITS);
                            }
                            if let Some(v) = spec_r.get_mut(k) {
                                *v = saturate_left_shift_alt(right, 2, DFRACT_BITS);
                            }
                        }
                    }
                } /* for ( band=0; band < max_sfb_ste; band++ ) */
                window += 1;
            } /* for ( groupwin=0; groupwin<pWindowGroupLength[group]; groupwin++, window++ ) */
        } /* for ( window = 0, group = 0; group < windowGroups; group++ ) */

    /* free scratch buffer */
    } else {
        /* MS stereo */

        let mut window: i32 = 0;
        for group in 0..window_groups {
            let group_mask: u8 = (1i32 << (group & 31)) as u8;

            for _groupwin in 0..wgl(group) {
                let left_off = spec_off(window, pl.granule_length);
                let right_off = spec_off(window, pr.granule_length);
                let mut band = 0;

                while band < max_sfb_ste_outside {
                    if jsd.ms_used.get(band as usize).copied().unwrap_or(0) & group_mask != 0 {
                        let si = (window * 16 + band) as usize;
                        if si >= 128 {
                            band += 1;
                            continue;
                        }
                        let l_scale0 = pl.p_dyn_data.a_sfb_scale[si] as i32;
                        let r_scale0 = pr.p_dyn_data.a_sfb_scale[si] as i32;
                        let mut common_scale =
                            if l_scale0 > r_scale0 { l_scale0 } else { r_scale0 };

                        /* ISO/IEC 14496-3 Chapter 4.6.8.1.1 : M/S joint channel coding can only
                         * be used if common_window is 1. */
                        fdk_assert!(
                            get_window_sequence(&pl.ics_info) == get_window_sequence(&pr.ics_info)
                        );
                        fdk_assert!(
                            get_window_shape(&pl.ics_info) == get_window_shape(&pr.ics_info)
                        );

                        common_scale += 1;
                        pl.p_dyn_data.a_sfb_scale[si] = common_scale as i16;
                        pr.p_dyn_data.a_sfb_scale[si] = common_scale as i16;

                        let l_scale = (DFRACT_BITS - 1).min(common_scale - l_scale0);
                        let r_scale = (DFRACT_BITS - 1).min(common_scale - r_scale0);

                        fdk_assert!(l_scale >= 0 && r_scale >= 0);

                        let offset_curr_band = off(band) as u32;
                        let offset_next_band = off(band + 1) as u32;

                        let lb = (left_off + offset_curr_band as usize).min(spec_l.len());
                        let rb = (right_off + offset_curr_band as usize).min(spec_r.len());
                        cjoint_stereo_generate_ms_output(
                            &mut spec_l[lb..],
                            &mut spec_r[rb..],
                            l_scale as u32,
                            r_scale as u32,
                            offset_next_band.wrapping_sub(offset_curr_band),
                        );
                    }
                    band += 1;
                }
                if scale_factor_bands_transmitted_l > scale_factor_bands_transmitted {
                    while band < scale_factor_bands_transmitted_l {
                        if jsd.ms_used.get(band as usize).copied().unwrap_or(0) & group_mask != 0 {
                            let si = (window * 16 + band) as usize;
                            if si < 128 {
                                pr.p_dyn_data.a_sfb_scale[si] = pl.p_dyn_data.a_sfb_scale[si];
                            }

                            for index in off(band)..off(band + 1) {
                                let left_coefficient =
                                    spec_l.get(left_off + index as usize).copied().unwrap_or(0);
                                /* FIXP_DBL rightCoefficient = (FIXP_DBL)0; */
                                if let Some(v) = spec_r.get_mut(right_off + index as usize) {
                                    *v = left_coefficient;
                                }
                            }
                        }
                        band += 1;
                    }
                } else if scale_factor_bands_transmitted_r > scale_factor_bands_transmitted {
                    while band < scale_factor_bands_transmitted_r {
                        if jsd.ms_used.get(band as usize).copied().unwrap_or(0) & group_mask != 0 {
                            let si = (window * 16 + band) as usize;
                            if si < 128 {
                                pl.p_dyn_data.a_sfb_scale[si] = pr.p_dyn_data.a_sfb_scale[si];
                            }

                            for index in off(band)..off(band + 1) {
                                /* FIXP_DBL leftCoefficient  = (FIXP_DBL)0; */
                                let right_coefficient =
                                    spec_r.get(right_off + index as usize).copied().unwrap_or(0);

                                if let Some(v) = spec_l.get_mut(left_off + index as usize) {
                                    *v = right_coefficient;
                                }
                                if let Some(v) = spec_r.get_mut(right_off + index as usize) {
                                    *v = right_coefficient.wrapping_neg();
                                }
                            }
                        }
                        band += 1;
                    }
                }
                window += 1;
            }
        }

        /* Reset MsUsed flags if no explicit signalling was transmitted. Necessary for intensity
        coding. PNS correlation signalling was mapped before calling CJointStereo_ApplyMS(). */
        if jsd.ms_mask_present == 2 {
            jsd.ms_used = [0; JointStereoMaximumBands as usize];
        }
    }
}

/// `CJointStereo_ApplyIS(pAacDecoderChannelInfo[2], pScaleFactorBandOffsets,
/// pWindowGroupLength, windowGroups, scaleFactorBandsTransmitted)`: apply intensity stereo.
///
/// `spec` is the spectrum of the element (the right channel at `spec[1024..]`), `com` the
/// element's `pComData` (`jointStereoData.MsUsed`).
pub fn cjoint_stereo_apply_is(
    p_aac_decoder_channel_info: &mut [&mut CAacDecoderChannelInfo],
    p_scale_factor_band_offsets: &[i16],
    p_window_group_length: &[u8],
    window_groups: i32,
    scale_factor_bands_transmitted: i32,
    spec: &mut [FixpDbl],
    com: &CAacDecoderCommonData,
) {
    let jsd = &com.joint_stereo_data;
    let offsets = p_scale_factor_band_offsets;
    let off = |b: i32| -> i32 { offsets.get(b as usize).copied().unwrap_or(0) as i32 };

    let (spec_l, spec_r) = {
        let n = spec.len();
        spec.split_at_mut(1024.min(n))
    };
    let (pl, pr) = {
        let (a, b) = p_aac_decoder_channel_info.split_at_mut(1);
        (&mut *a[0], &mut *b[0])
    };

    let mut window: i32 = 0;
    for group in 0..window_groups {
        let group_mask: u8 = (1i32 << (group & 31)) as u8;

        for _groupwin in 0..p_window_group_length.get(group as usize).copied().unwrap_or(0) {
            let left_off = spec_off(window, pl.granule_length);
            let right_off = spec_off(window, pr.granule_length);

            for band in 0..scale_factor_bands_transmitted {
                let gi = (group * 16 + band) as usize;
                let wi = (window * 16 + band) as usize;
                let Some(&code_book) = pr.p_dyn_data.a_code_book.get(gi) else { continue };
                if (code_book == INTENSITY_HCB) || (code_book == INTENSITY_HCB2) {
                    let band_scale: i32 = -(pr.p_dyn_data.a_scale_factor[gi] as i32 + 100);

                    let msb = band_scale >> 2;
                    let lsb = band_scale & 0x03;

                    /* exponent of MantissaTable[lsb][0] is 1, thus msb+1 below. */
                    let mut scale: FixpDbl = MANTISSA_TABLE[lsb as usize][0];

                    /* ISO/IEC 14496-3 Chapter 4.6.8.2.3 : The use of intensity stereo coding is
                    signaled by the use of the pseudo codebooks INTENSITY_HCB and INTENSITY_HCB2
                    (15 and 14) only in the right channel of a channel_pair_element() having a
                    common ics_info() (common_window == 1). */
                    fdk_assert!(
                        get_window_sequence(&pl.ics_info) == get_window_sequence(&pr.ics_info)
                    );
                    fdk_assert!(get_window_shape(&pl.ics_info) == get_window_shape(&pr.ics_info));

                    if wi < 128 {
                        pr.p_dyn_data.a_sfb_scale[wi] =
                            (pl.p_dyn_data.a_sfb_scale[wi] as i32 + msb + 1) as i16;
                    }

                    if jsd.ms_used.get(band as usize).copied().unwrap_or(0) & group_mask != 0 {
                        if code_book == INTENSITY_HCB {
                            /* _NOT_ in-phase */
                            scale = -scale;
                        }
                    } else {
                        if code_book == INTENSITY_HCB2 {
                            /* out-of-phase */
                            scale = -scale;
                        }
                    }

                    for index in off(band)..off(band + 1) {
                        let l = spec_l.get(left_off + index as usize).copied().unwrap_or(0);
                        if let Some(v) = spec_r.get_mut(right_off + index as usize) {
                            *v = f_mult(l, scale);
                        }
                    }
                }
            }
            window += 1;
        }
    }
}
