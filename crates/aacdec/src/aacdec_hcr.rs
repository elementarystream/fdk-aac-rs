// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACdec/src/aacdec_hcr.h, libAACdec/src/aacdec_hcr.cpp.

//! Port of `aacdec_hcr.h`: HCR (Huffman codeword reordering) decoder: initialization,
//! preprocessing of the HCR side info and decoding of the priority codewords (PCWs).
//!
//! # Memory
//!
//! The HCR state ([`CErHcrInfo`], `pComData->overlay.aac.erHcrInfo` in C) stores no pointers
//! (see [`crate::aacdec_hcr_types`]): the functions that reach the spectrum of the channel
//! through `decInOut.pQuantizedSpectralCoefficientsBase` or `nonPcwSideinfo.pResultBase` take
//! it as `spec` (the channel's `pSpectralCoefficient`, at least 1024 values), and the codebooks
//! and line counts of the sections (`decInOut.pCodebook`, `decInOut.pNumLineInSect`) are taken
//! from the channel's `pDynData->specificTo.aac`.
//!
//! The file-local (`static`) functions of `aacdec_hcr.cpp` are public here so that the
//! differential tests can reach them.
//!
//! # Indices
//!
//! An index past `aCodeBooks4Hcr` reaches the next field of the C structure; reproduced from
//! the C layout ([`write_code_books4_hcr`]).
//!
//! Elsewhere, indices outside the arrays, where the reference has no defined result, read 0 and
//! the writes are skipped; the loop of `HcrSortCodebookAndNumCodewordInSection()` stops at the end
//! of the array, and the codebooks of the extended sorted sections are taken modulo 32 (they are
//! always below 32) (PORTING.md, "Deviations from the reference").

use fdk_aac_rs_fdk::audio::{Mp4ElementId, ID_CCE, ID_CPE, ID_LFE, ID_SCE};
use fdk_aac_rs_fdk::bitstream::{
    fdk_get_valid_bits, fdk_push_bi_directional, fdk_read_bits, fdk_sync_cache, FdkBitStream,
};
use fdk_aac_rs_fdk::common_fix::{f_abs, f_min, fix_max, fixp_abs, FixpDbl};

use crate::aac_rom::{
    A_CB_PRIORITY, A_DIM_CB, A_DIM_CB_SHIFT, A_HUFF_TABLE, A_LARGEST_ABSOLUTE_VALUE, A_MAX_CW_LEN,
    A_MAX_OF_CB_PAIR, A_MIN_OF_CB_PAIR, A_QUANT_TABLE, A_SIGN_CB,
};
use crate::aacdec_hcr_bit::hcr_get_a_bit_from_bitstream_ltr;
use crate::aacdec_hcr_types::*;
use crate::aacdec_hcrs::{decode_non_pcws, DIMENSION_OF_ESCAPE_CODEBOOK};
use crate::block::get_scale_from_value;
use crate::channelinfo::{
    get_scale_factor_band_offsets, get_scale_factor_bands_transmitted, get_window_group_length,
    get_window_groups, is_long_block, CAacDecoderChannelInfo, CAacDecoderDynamicDataAac,
    SamplingRateInfo, BOOKSCL, INTENSITY_HCB, INTENSITY_HCB2, MAX_QUANTIZED_VALUE, NOISE_HCB,
    ZERO_HCB,
};
use crate::overlapadd::{spec, spec_long};

/// `a[i]`, or the default value (0) for an index outside the array (see the
/// [module documentation](self#indices)).
#[inline(always)]
pub(crate) fn rd<T: Copy + Default>(a: &[T], i: usize) -> T {
    a.get(i).copied().unwrap_or_default()
}

/// `a[i] = v`, skipped for an index outside the array (see the
/// [module documentation](self#indices)).
#[inline(always)]
pub(crate) fn wr<T>(a: &mut [T], i: usize, v: T) {
    if let Some(x) = a.get_mut(i) {
        *x = v;
    }
}

/// `aCodeBooks4Hcr[idx] = v`. Index `MAX_SFB_HCR` (256) reaches the next field of the C structure
/// (`CAacDecoderDynamicData::specificTo.aac`), the low byte of `lenOfReorderedSpectralData`;
/// reproduced from the C layout (checked by the layout test of the differential tests). Higher
/// indices are skipped.
pub fn write_code_books4_hcr(aac: &mut CAacDecoderDynamicDataAac, idx: usize, v: u8) {
    if idx < MAX_SFB_HCR as usize {
        aac.a_code_books4_hcr[idx] = v;
    } else if idx == MAX_SFB_HCR as usize {
        aac.len_of_reordered_spectral_data =
            ((aac.len_of_reordered_spectral_data as u16 & 0xFF00) | v as u16) as i16;
    }
}

/// `errDetectorInHcrSideinfoShrt(cb, numLine, errorWord)`: checks if codebook and numSect are
/// within allowed range (short only).
pub fn err_detector_in_hcr_sideinfo_shrt(cb: i8, num_line: i16, error_word: &mut u32) {
    if (cb as i32) < ZERO_HCB as i32 || cb as i32 >= MAX_CB_CHECK || cb as i32 == BOOKSCL as i32 {
        *error_word |= CB_OUT_OF_RANGE_SHORT_BLOCK;
    }
    if num_line < 0 || num_line > 1024 {
        *error_word |= LINE_IN_SECT_OUT_OF_RANGE_SHORT_BLOCK;
    }
}

/// `errDetectorInHcrLengths(lengthOfLongestCodeword, lengthOfReorderedSpectralData,
/// errorWord)`: checks both HCR lengths.
pub fn err_detector_in_hcr_lengths(
    length_of_longest_codeword: i8,
    length_of_reordered_spectral_data: i16,
    error_word: &mut u32,
) {
    if (length_of_reordered_spectral_data as i32) < length_of_longest_codeword as i32 {
        *error_word |= HCR_SI_LENGTHS_FAILURE;
    }
}

/// `CHcr_Read(bs, pAacDecoderChannelInfo, globalHcrType)`: decodes (and adapts if necessary)
/// the two HCR side info components `reordered_spectral_data_length` and
/// `longest_codeword_length`.
pub fn chcr_read(
    bs: &mut FdkBitStream,
    p_aac_decoder_channel_info: &mut CAacDecoderChannelInfo,
    global_hcr_type: Mp4ElementId,
) {
    let aac = &mut p_aac_decoder_channel_info.p_dyn_data.specific_to.aac;

    aac.len_of_reordered_spectral_data = 0;
    aac.len_of_longest_codeword = 0;

    /* ------- SI-Value No 1 ------- */
    let leng_of_reordered_spectral_data: i16 = (fdk_read_bits(bs, 14) as i32 + ERROR_LORSD) as i16;
    if global_hcr_type == ID_CPE {
        if (leng_of_reordered_spectral_data >= 0)
            && (leng_of_reordered_spectral_data as i32 <= CPE_TOP_LENGTH)
        {
            /* the decoded value is within range */
            aac.len_of_reordered_spectral_data = leng_of_reordered_spectral_data;
        } else {
            if leng_of_reordered_spectral_data as i32 > CPE_TOP_LENGTH {
                aac.len_of_reordered_spectral_data = CPE_TOP_LENGTH as i16; /* use valid maximum */
            }
        }
    } else if global_hcr_type == ID_SCE || global_hcr_type == ID_LFE || global_hcr_type == ID_CCE {
        if (leng_of_reordered_spectral_data >= 0)
            && (leng_of_reordered_spectral_data as i32 <= SCE_TOP_LENGTH)
        {
            /* the decoded value is within range */
            aac.len_of_reordered_spectral_data = leng_of_reordered_spectral_data;
        } else {
            if leng_of_reordered_spectral_data as i32 > SCE_TOP_LENGTH {
                aac.len_of_reordered_spectral_data = SCE_TOP_LENGTH as i16; /* use valid maximum */
            }
        }
    }

    /* ------- SI-Value No 2 ------- */
    let leng_of_longest_codeword: i8 = (fdk_read_bits(bs, 6) as i32 + ERROR_LOLC) as i8;
    if (leng_of_longest_codeword >= 0)
        && (leng_of_longest_codeword as i32 <= LEN_OF_LONGEST_CW_TOP_LENGTH)
    {
        /* the decoded value is within range */
        aac.len_of_longest_codeword = leng_of_longest_codeword;
    } else {
        if leng_of_longest_codeword as i32 > LEN_OF_LONGEST_CW_TOP_LENGTH {
            /* use valid maximum */
            aac.len_of_longest_codeword = LEN_OF_LONGEST_CW_TOP_LENGTH as i8;
        }
    }
}

/// `HcrInit(pHcr, pAacDecoderChannelInfo, pSamplingRateInfo, bs)`: sets up HCR; must be called
/// before every call to [`hcr_decoder`]. For short blocks a sorting algorithm is applied to get
/// the side info in the order that HCR could assemble the qsc's as if it is a long block.
///
/// Returns the error log.
pub fn hcr_init(
    p_hcr: &mut CErHcrInfo,
    p_aac_decoder_channel_info: &mut CAacDecoderChannelInfo,
    p_sampling_rate_info: &SamplingRateInfo,
    bs: &mut FdkBitStream,
) -> u32 {
    let p_ics_info = &p_aac_decoder_channel_info.ics_info;
    let p_dyn_data = &mut *p_aac_decoder_channel_info.p_dyn_data;
    let mut num_section: i16;
    let mut cb: i8;
    let mut num_line: i32;

    p_hcr.dec_in_out.length_of_reordered_spectral_data =
        p_dyn_data.specific_to.aac.len_of_reordered_spectral_data;
    p_hcr.dec_in_out.length_of_longest_codeword =
        p_dyn_data.specific_to.aac.len_of_longest_codeword;
    p_hcr.dec_in_out.quantized_spectral_coefficients_idx = 0;
    p_hcr.dec_in_out.num_section = p_dyn_data.specific_to.aac.number_section as i16;
    p_hcr.dec_in_out.error_log = 0;

    fdk_sync_cache(bs);
    p_hcr.dec_in_out.bitstream_anchor = fdk_get_valid_bits(bs) as i32;

    if is_long_block(p_ics_info) == 0
    /* short block */
    {
        let p_code_book = &p_dyn_data.a_code_book; /* in */
        let aac = &mut p_dyn_data.specific_to.aac; /* out: aNumLineInSec4Hcr, aCodeBooks4Hcr */
        let band_offsets = get_scale_factor_band_offsets(p_ics_info, p_sampling_rate_info); /* aux */
        let num_of_groups = get_window_groups(p_ics_info) as i8;
        let mut num_lines_in_sec_idx = 0usize;
        let mut code_bk_idx = 0usize;
        let mut cb_prev: i8;

        num_line = 0;
        num_section = 0;
        cb = p_code_book[0] as i8;
        cb_prev = p_code_book[0] as i8;

        /* convert HCR-sideinfo into a unitwise manner: When the cb changes, a new section
        starts */

        write_code_books4_hcr(aac, code_bk_idx, cb_prev as u8);
        code_bk_idx += 1;

        let max_band = get_scale_factor_bands_transmitted(p_ics_info) as i16;
        let mut band: i16 = 0;
        while band < max_band {
            /* from low to high sfbs i.e. from low to high frequencies */
            /* get the number of units in current sfb */
            let num_unit_in_band: i8 = (((rd(band_offsets, band as usize + 1) as i32)
                - (rd(band_offsets, band as usize) as i32))
                >> FOUR_LOG_DIV_TWO_LOG) as i8;
            let mut cnt_unit_in_band = num_unit_in_band;
            while cnt_unit_in_band != 0 {
                /* for every unit in the band */
                let mut group: i8 = 0;
                while group < num_of_groups {
                    let win_group_len = get_window_group_length(p_ics_info, group as i32) as i8;
                    let mut group_win = win_group_len;
                    while group_win != 0 {
                        cb = rd(p_code_book, (group as i32 * 16 + band as i32) as usize) as i8;
                        if cb != cb_prev {
                            err_detector_in_hcr_sideinfo_shrt(
                                cb,
                                num_line as i16,
                                &mut p_hcr.dec_in_out.error_log,
                            );
                            if p_hcr.dec_in_out.error_log != 0 {
                                return p_hcr.dec_in_out.error_log;
                            }
                            write_code_books4_hcr(aac, code_bk_idx, cb as u8);
                            code_bk_idx += 1;
                            wr(
                                &mut aac.a_num_line_in_sec4_hcr,
                                num_lines_in_sec_idx,
                                num_line as i16,
                            );
                            num_lines_in_sec_idx += 1;
                            num_section = num_section.wrapping_add(1);

                            cb_prev = cb;
                            num_line = LINES_PER_UNIT;
                        } else {
                            num_line += LINES_PER_UNIT;
                        }
                        group_win = group_win.wrapping_sub(1);
                    }
                    group = group.wrapping_add(1);
                }
                cnt_unit_in_band = cnt_unit_in_band.wrapping_sub(1);
            }
            band += 1;
        }

        num_section = num_section.wrapping_add(1);

        err_detector_in_hcr_sideinfo_shrt(cb, num_line as i16, &mut p_hcr.dec_in_out.error_log);
        if num_section <= 0 || num_section as i32 > 1024 / 2 {
            p_hcr.dec_in_out.error_log |= NUM_SECT_OUT_OF_RANGE_SHORT_BLOCK;
        }
        err_detector_in_hcr_lengths(
            p_hcr.dec_in_out.length_of_longest_codeword,
            p_hcr.dec_in_out.length_of_reordered_spectral_data,
            &mut p_hcr.dec_in_out.error_log,
        );
        if p_hcr.dec_in_out.error_log != 0 {
            return p_hcr.dec_in_out.error_log;
        }

        write_code_books4_hcr(aac, code_bk_idx, cb as u8);
        wr(&mut aac.a_num_line_in_sec4_hcr, num_lines_in_sec_idx, num_line as i16);
        p_hcr.dec_in_out.num_section = num_section;
    } else
    /* end short block prepare SI */
    {
        /* long block */
        let aac = &p_dyn_data.specific_to.aac;
        err_detector_in_hcr_lengths(
            p_hcr.dec_in_out.length_of_longest_codeword,
            p_hcr.dec_in_out.length_of_reordered_spectral_data,
            &mut p_hcr.dec_in_out.error_log,
        );
        num_section = p_hcr.dec_in_out.num_section;
        if num_section <= 0 || num_section > 64 {
            p_hcr.dec_in_out.error_log |= NUM_SECT_OUT_OF_RANGE_LONG_BLOCK;
            num_section = 0;
        }

        for i in 0..num_section as usize {
            cb = aac.a_code_books4_hcr[i] as i8;

            if (cb as i32) < ZERO_HCB as i32
                || cb as i32 >= MAX_CB_CHECK
                || cb as i32 == BOOKSCL as i32
            {
                p_hcr.dec_in_out.error_log |= CB_OUT_OF_RANGE_LONG_BLOCK;
            }

            num_line = aac.a_num_line_in_sec4_hcr[i] as i32;
            /* FDK_ASSERT(numLine > 0); */

            if (num_line <= 0) || (num_line > 1024) {
                p_hcr.dec_in_out.error_log |= LINE_IN_SECT_OUT_OF_RANGE_LONG_BLOCK;
            }
        }
        if p_hcr.dec_in_out.error_log != 0 {
            return p_hcr.dec_in_out.error_log;
        }
    }

    let code_bk = &mut p_dyn_data.specific_to.aac.a_code_books4_hcr;
    for i in 0..(num_section.max(0) as usize) {
        if let Some(c) = code_bk.get_mut(i) {
            if (*c == NOISE_HCB) || (*c == INTENSITY_HCB2) || (*c == INTENSITY_HCB) {
                *c = 0;
            }
        }
    }

    /* HCR-sideinfo-input is complete and seems to be valid */

    p_hcr.dec_in_out.error_log
}

/// `HcrDecoder(pHcr, pAacDecoderChannelInfo, pSamplingRateInfo, bs)`: decodes the codewords of
/// the spectral coefficients from the bitstream according to the HCR algorithm and stores the
/// quantized spectral coefficients in correct order in the output buffer.
///
/// `spec` is `pAacDecoderChannelInfo->pSpectralCoefficient` (at least 1024 values). Returns the
/// error log.
pub fn hcr_decoder(
    p_hcr: &mut CErHcrInfo,
    p_aac_decoder_channel_info: &mut CAacDecoderChannelInfo,
    p_sampling_rate_info: &SamplingRateInfo,
    bs: &mut FdkBitStream,
    spec: &mut [FixpDbl],
) -> u32 {
    let save_bit_cnt = fdk_get_valid_bits(bs) as i32; /* save bitstream position */

    {
        let aac = &p_aac_decoder_channel_info.p_dyn_data.specific_to.aac;
        hcr_calc_num_codeword(p_hcr, &aac.a_code_books4_hcr, &aac.a_num_line_in_sec4_hcr);

        hcr_sort_codebook_and_num_codeword_in_section(p_hcr, &aac.a_code_books4_hcr);
    }

    hcr_prepare_segmentation_grid(p_hcr);

    hcr_extended_section_info(p_hcr);

    if (p_hcr.dec_in_out.error_log & HCR_FATAL_PCW_ERROR_MASK) != 0 {
        /* sideinfo is massively corrupt, return from HCR without having decoded anything */
        return p_hcr.dec_in_out.error_log;
    }

    {
        let si = &mut p_hcr.section_info;
        derive_number_of_extended_sorted_sections_in_sets(
            p_hcr.segment_info.num_segment,
            &si.p_num_extended_sorted_codeword_in_section,
            si.num_extended_sorted_codeword_in_section_idx,
            &mut si.p_num_extended_sorted_sections_in_sets,
            si.num_extended_sorted_sections_in_sets_idx,
        );
    }

    /* store */
    let p_tmp1 = p_hcr.section_info.num_extended_sorted_codeword_in_section_idx;
    let p_tmp2 = p_hcr.section_info.extended_sorted_codebook_idx;
    let p_tmp3 = p_hcr.section_info.num_extended_sorted_sections_in_sets_idx;
    let p_tmp4 = p_hcr.dec_in_out.quantized_spectral_coefficients_idx;
    let p_tmp5 = p_hcr.section_info.max_len_of_cb_in_ext_srt_sec_idx;

    /* ------- decode meaningful PCWs ------ */
    decode_pcws(bs, p_hcr, spec);

    if (p_hcr.dec_in_out.error_log & HCR_FATAL_PCW_ERROR_MASK) == 0 {
        /* ------ decode the non-PCWs -------- */
        decode_non_pcws(bs, p_hcr, spec);
    }

    err_detect_within_segmentation_final(p_hcr);

    /* restore */
    p_hcr.section_info.num_extended_sorted_codeword_in_section_idx = p_tmp1;
    p_hcr.section_info.extended_sorted_codebook_idx = p_tmp2;
    p_hcr.section_info.num_extended_sorted_sections_in_sets_idx = p_tmp3;
    p_hcr.dec_in_out.quantized_spectral_coefficients_idx = p_tmp4;
    p_hcr.section_info.max_len_of_cb_in_ext_srt_sec_idx = p_tmp5;

    hcr_reorder_quantized_spectral_coefficients(
        p_hcr,
        p_aac_decoder_channel_info,
        p_sampling_rate_info,
        spec,
    );

    /* restore bitstream position */
    let bit_cnt_offst = (fdk_get_valid_bits(bs) as i32).wrapping_sub(save_bit_cnt);
    if bit_cnt_offst != 0 {
        fdk_push_bi_directional(bs, bit_cnt_offst);
    }

    p_hcr.dec_in_out.error_log
}

/// `HcrReorderQuantizedSpectralCoefficients(pHcr, pAacDecoderChannelInfo, pSamplingRateInfo)`:
/// reorders the quantized spectral coefficients sectionwise for long and short blocks and
/// compares them to the LAV (largest absolute value of the current codebook); for short blocks,
/// it also deinterleaves the units and derives the scaling (as the plain Huffman decoder does).
pub fn hcr_reorder_quantized_spectral_coefficients(
    p_hcr: &mut CErHcrInfo,
    p_aac_decoder_channel_info: &mut CAacDecoderChannelInfo,
    p_sampling_rate_info: &SamplingRateInfo,
    spec_buf: &mut [FixpDbl],
) {
    let mut lav_error_cnt: u16 = 0;

    let num_section = p_hcr.dec_in_out.num_section as u32;
    let mut p_quantized_spectral_coefficients = spec_long();
    let sec = &p_hcr.section_info;
    let mut sorted_codebook_idx = 0usize;
    let mut num_sorted_cw_idx = 0usize;
    let mut reorder_offset_idx = 0usize;
    let mut p_temp_values = [0 as FixpDbl; 1024];

    /* long and short: check if decoded huffman-values (quantized spectral coefficients) are
    within range */
    let mut i = num_section;
    while i != 0 {
        let sorted_codebook = rd(&sec.p_sorted_codebook, sorted_codebook_idx) as usize;
        let num_spectral_values_in_section: u16 =
            ((rd(&sec.p_num_sorted_codeword_in_section, num_sorted_cw_idx) as u32)
                << rd(&A_DIM_CB_SHIFT, sorted_codebook)) as u16;
        num_sorted_cw_idx += 1;
        let mut p_te_va = rd(&sec.p_reorder_offset, reorder_offset_idx) as usize;
        reorder_offset_idx += 1;
        let lav = rd(&A_LARGEST_ABSOLUTE_VALUE, sorted_codebook) as u32;
        for _ in 0..num_spectral_values_in_section {
            let qsc: i32 = rd(spec_buf, p_quantized_spectral_coefficients);
            p_quantized_spectral_coefficients += 1;
            let abs_qsc = f_abs(qsc) as u32;
            if abs_qsc <= lav {
                wr(&mut p_temp_values, p_te_va, qsc); /* the qsc value is within range */
            } else {
                /* line is too high .. */
                if abs_qsc == Q_VALUE_INVALID as u32 {
                    /* .. because of previous marking --> dont set LAV flag (would be confusing),
                    just copy out the already marked value */
                    wr(&mut p_temp_values, p_te_va, qsc);
                } else {
                    /* .. because a too high value was decoded for this cb --> set LAV flag */
                    wr(&mut p_temp_values, p_te_va, Q_VALUE_INVALID);
                    lav_error_cnt = lav_error_cnt.wrapping_add(1);
                }
            }
            p_te_va += 1;
        }
        sorted_codebook_idx += 1;
        i -= 1;
    }

    let p_ics_info = &p_aac_decoder_channel_info.ics_info;
    if is_long_block(p_ics_info) == 0 {
        let granule_length = p_aac_decoder_channel_info.granule_length;
        let p_dyn_data = &mut *p_aac_decoder_channel_info.p_dyn_data;
        let band_offsets = get_scale_factor_band_offsets(p_ics_info, p_sampling_rate_info);

        /* deinterleave unitwise for short blocks */
        for window in 0..8i32 {
            let mut p_out = spec(window, granule_length);
            for i in 0..LINES_PER_UNIT_GROUP as usize {
                /* distance of lines between unit groups has to be constant for every
                framelength (32)! */
                let mut p_te_va = ((window as usize) << FOUR_LOG_DIV_TWO_LOG) + i * 32;
                for _ in 0..LINES_PER_UNIT {
                    wr(spec_buf, p_out, rd(&p_temp_values, p_te_va));
                    p_out += 1;
                    p_te_va += 1;
                }
            }
        }

        /* short blocks only */
        /* derive global scaling-value for every sfb and every window (as it is done in
        plain-huffman-decoder at short blocks) */
        let mut groupoffset: i8 = 0;

        let num_win_group = get_window_groups(p_ics_info) as i8;
        let num_sfb_transm = get_scale_factor_bands_transmitted(p_ics_info) as i8;

        let mut group: i8 = 0;
        while group < num_win_group {
            let win_group_len = get_window_group_length(p_ics_info, group as i32) as i8;
            let mut band: i8 = 0;
            while band < num_sfb_transm {
                let interm: i16 = (group as i32 * 16 + band as i32) as i16;
                let sf = rd(&p_dyn_data.a_scale_factor, interm as usize) as i32;
                let msb: i32 = sf >> 2;
                let lsb: i32 = sf & 3;
                let mut groupwin: i8 = 0;
                while groupwin < win_group_len {
                    let window: i8 = groupoffset.wrapping_add(groupwin);
                    let p_bak = spec(window as i32, granule_length);
                    let mut loc_max: FixpDbl = 0;
                    let mut index: i16 = rd(band_offsets, band as usize);
                    let end = rd(band_offsets, band as usize + 1);
                    while index < end {
                        let mut p_te_va = p_bak.wrapping_add(index as isize as usize);
                        for _ in 0..LINES_PER_UNIT {
                            let v = rd(spec_buf, p_te_va);
                            p_te_va = p_te_va.wrapping_add(1);
                            let tmp = if v < 0 { v.wrapping_neg() } else { v };
                            loc_max = fix_max(tmp, loc_max);
                        }
                        index = index.wrapping_add(LINES_PER_UNIT as i16);
                    }
                    if fixp_abs(loc_max) > MAX_QUANTIZED_VALUE as FixpDbl {
                        loc_max = MAX_QUANTIZED_VALUE as FixpDbl;
                    }
                    /* save global scale maxima in this sfb */
                    wr(
                        &mut p_dyn_data.a_sfb_scale,
                        (window as i32 * 16 + band as i32) as usize,
                        (msb - get_scale_from_value(loc_max, lsb as u32)) as i16,
                    );
                    groupwin = groupwin.wrapping_add(1);
                }
                band = band.wrapping_add(1);
            }
            groupoffset = groupoffset.wrapping_add(win_group_len);
            group = group.wrapping_add(1);
        }
    } else {
        /* copy straight for long-blocks */
        let n = spec_buf.len().min(1024);
        spec_buf[spec_long()..n].copy_from_slice(&p_temp_values[..n]);
    }

    if lav_error_cnt != 0 {
        p_hcr.dec_in_out.error_log |= LAV_VIOLATION;
    }
}

/// `HcrCalcNumCodeword(pHcr)`: calculates the number of codewords for each section
/// (`numCodewordInSection`) and the number of codewords for all sections (`numCodeword`). For
/// zero and intensity codebooks an entry is also made in `numCodewordInSection`, assuming a two
/// tuples codebook (needed for the base addresses of the reordering); `numCodeword` contains only
/// the codewords that are really in the bitstream.
///
/// `p_codebook` and `p_num_line_in_section` are `decInOut.pCodebook` and
/// `decInOut.pNumLineInSect` (the channel's `aCodeBooks4Hcr` and `aNumLineInSec4Hcr`).
pub fn hcr_calc_num_codeword(
    p_hcr: &mut CErHcrInfo,
    p_codebook: &[u8],
    p_num_line_in_section: &[i16],
) {
    let num_section = p_hcr.dec_in_out.num_section as u32;
    let p_num_codeword_in_section = &mut p_hcr.section_info.p_num_codeword_in_section;

    let mut num_codeword: u32 = 0;
    let mut hcr_section = num_section as i32;
    let mut idx = 0usize;
    while hcr_section != 0 && idx < MAX_SFB_HCR as usize {
        let cb = rd(p_codebook, idx);
        let n =
            ((rd(p_num_line_in_section, idx) as i32) >> rd(&A_DIM_CB_SHIFT, cb as usize)) as u16;
        p_num_codeword_in_section[idx] = n;
        if cb != 0 {
            num_codeword = num_codeword.wrapping_add(n as u32);
        }
        idx += 1;
        hcr_section = hcr_section.wrapping_sub(1);
    }
    p_hcr.section_info.num_codeword = num_codeword;
}

/// `HcrSortCodebookAndNumCodewordInSection(pHcr)`: calculates the number of sorted codebooks
/// and sorts the codebooks and `numCodewordInSection` according to the priority.
///
/// `p_codebook` is `decInOut.pCodebook` (the channel's `aCodeBooks4Hcr`).
// `searchStart` changes inside `for (i = searchStart; ...)` only right before its `break`, as in C.
#[allow(clippy::mut_range_bound)]
pub fn hcr_sort_codebook_and_num_codeword_in_section(p_hcr: &mut CErHcrInfo, p_codebook: &[u8]) {
    let num_section = p_hcr.dec_in_out.num_section as u32;
    // The C loops run over numSection (1 to 256 after HcrInit()); the port bounds them by the
    // arrays (see the module documentation).
    let n = (num_section as usize).min(MAX_SFB_HCR as usize);
    let si = &mut p_hcr.section_info;

    /* calculate *pNumSortedSection and store the priorities in array pSortedCdebook */
    let mut num_zero_section: u32 = 0;
    for i in 0..n {
        let pr = rd(&A_CB_PRIORITY, rd(p_codebook, i) as usize);
        if pr == 0 {
            num_zero_section += 1;
        }
        si.p_sorted_codebook[i] = pr;
    }
    /* numSortedSection contains no zero or intensity section */
    si.num_sorted_section = num_section.wrapping_sub(num_zero_section);

    /* sort priorities of the codebooks in array pSortedCdebook[] */
    if n > 1 {
        let mut counter = n - 1;
        for _ in 0..n - 1 {
            for i in 0..counter {
                /* swap priorities */
                if si.p_sorted_codebook[i + 1] > si.p_sorted_codebook[i] {
                    si.p_sorted_codebook.swap(i, i + 1);
                }
            }
            counter -= 1;
        }
    }

    /* clear codebookSwitch array */
    for i in 0..n {
        si.p_codebook_switch[i] = 0;
    }

    /* sort sectionCodebooks and numCodwordsInSection and calculate pReorderOffst[j] */
    let mut search_start = 0usize;
    for j in 0..n {
        for i in search_start..n {
            let pr = si.p_sorted_codebook[j] as usize;
            let cb = rd(p_codebook, i);
            if si.p_codebook_switch[i] == 0
                && (rd(&A_MIN_OF_CB_PAIR, pr) == cb || rd(&A_MAX_OF_CB_PAIR, pr) == cb)
            {
                si.p_codebook_switch[i] = 1;
                si.p_sorted_codebook[j] = cb; /* sort codebook */
                /* sort NumCodewordInSection */
                si.p_num_sorted_codeword_in_section[j] = si.p_num_codeword_in_section[i];

                let mut start_offset: u32 = 0;
                for k in 0..i {
                    /* make entry in pReorderOffst */
                    start_offset = start_offset.wrapping_add(
                        (si.p_num_codeword_in_section[k] as u32)
                            << rd(&A_DIM_CB_SHIFT, rd(p_codebook, k) as usize),
                    );
                }
                /* offset for reordering the codewords */
                si.p_reorder_offset[j] = start_offset as u16;

                if i == search_start {
                    // while (pCodebookSwitch[k++] == 1) searchStart++; -- the port stops at the
                    // end of the array; searchStart is only used again if a section is left,
                    // and the loop stops before it.
                    let mut k = i;
                    while k < MAX_SFB_HCR as usize && si.p_codebook_switch[k] == 1 {
                        k += 1;
                        search_start += 1;
                    }
                }
                break;
            }
        }
    }
}

/// `HcrPrepareSegmentationGrid(pHcr)`: calculates the segmentation (`numSegment`,
/// `leftStartOfSegment`, `rightStartOfSegment` and `remainingBitsInSegment`), a kind of overlay
/// grid for the bitstream block holding the HCR-encoded quantized spectral coefficients.
pub fn hcr_prepare_segmentation_grid(p_hcr: &mut CErHcrInfo) {
    let mut num_segment: u16 = 0;
    let mut segment_start: i32 = 0;
    let mut end_flag: u8 = 0;

    let length_of_longest_codeword = p_hcr.dec_in_out.length_of_longest_codeword;
    let length_of_reordered_spectral_data = p_hcr.dec_in_out.length_of_reordered_spectral_data;
    let num_sorted_section = p_hcr.section_info.num_sorted_section;
    let sc = &p_hcr.section_info;
    let sg = &mut p_hcr.segment_info;
    let mut sorted_idx = 0usize;
    let mut num_sorted_cw_idx = 0usize;
    // pLeftStartOfSegment, pRightStartOfSegment, pRemainingBitsInSegment
    let mut p: isize = 0;

    let mut i: u16 = num_sorted_section as u16;
    while i != 0 {
        let sorted_codebook = rd(&sc.p_sorted_codebook, sorted_idx);
        sorted_idx += 1;
        let segment_width: u8 = f_min(
            rd(&A_MAX_CW_LEN, sorted_codebook as usize) as i32,
            length_of_longest_codeword as i32,
        ) as u8;

        let mut j: u16 = rd(&sc.p_num_sorted_codeword_in_section, num_sorted_cw_idx);
        while j != 0 {
            /* width allows a new segment */
            let intermediate_result = segment_start;
            if (segment_start + segment_width as i32) <= length_of_reordered_spectral_data as i32 {
                /* store segment start, segment length and increment the number of segments */
                wr(&mut sg.p_left_start_of_segment, p as usize, intermediate_result);
                wr(
                    &mut sg.p_right_start_of_segment,
                    p as usize,
                    intermediate_result + segment_width as i32 - 1,
                );
                wr(&mut sg.p_remaining_bits_in_segment, p as usize, segment_width as i8);
                p += 1;
                segment_start += segment_width as i32;
                num_segment = num_segment.wrapping_add(1);
            }
            /* width does not allow a new segment */
            else {
                /* correct the last segment length */
                p -= 1;
                segment_start = rd(&sg.p_left_start_of_segment, p as usize);

                let last_segment_width: u8 =
                    (length_of_reordered_spectral_data as i32 - segment_start) as u8;
                wr(&mut sg.p_remaining_bits_in_segment, p as usize, last_segment_width as i8);
                wr(
                    &mut sg.p_right_start_of_segment,
                    p as usize,
                    segment_start + last_segment_width as i32 - 1,
                );
                end_flag = 1;
                break;
            }
            j -= 1;
        }
        num_sorted_cw_idx += 1;
        if end_flag != 0 {
            break;
        }
        i -= 1;
    }
    p_hcr.segment_info.num_segment = num_segment as u32;
}

/// `HcrExtendedSectionInfo(pHcr)`: adapts the sorted section boundaries to the boundaries of the
/// segmentation. If a section does not fit completely into the current segment, it is split into
/// two so called extended sections (`pNumExtendedSortedCodewordInSection` and
/// `pExtendedSortedCodebook` are updated).
pub fn hcr_extended_section_info(p_hcr: &mut CErHcrInfo) {
    let mut srt_sec_cnt: u32 = 0; /* counter for sorted sections */
    let mut x_srt_sc_cnt: u32 = 0; /* counter for extended sorted sections */

    let num_sorted_section = p_hcr.section_info.num_sorted_section;
    let num_segment = p_hcr.segment_info.num_segment;
    let length_of_longest_codeword = p_hcr.dec_in_out.length_of_longest_codeword;
    let si = &mut p_hcr.section_info;

    // An index outside pNumSortedCodewordInSection reads 0; the value is not used (the loop
    // ends).
    let mut remain_num_cw_in_sort_sec =
        rd(&si.p_num_sorted_codeword_in_section, srt_sec_cnt as usize) as u32;
    let mut in_segment_remain_num_cw = num_segment;

    while srt_sec_cnt < num_sorted_section {
        let x = x_srt_sc_cnt as usize;
        if in_segment_remain_num_cw < remain_num_cw_in_sort_sec {
            si.p_num_extended_sorted_codeword_in_section[x] = in_segment_remain_num_cw as u16;
            si.p_extended_sorted_codebook[x] = rd(&si.p_sorted_codebook, srt_sec_cnt as usize);

            remain_num_cw_in_sort_sec -= in_segment_remain_num_cw;
            in_segment_remain_num_cw = num_segment;
            /* data of a sorted section was not integrated in extended sorted section */
        } else if in_segment_remain_num_cw == remain_num_cw_in_sort_sec {
            si.p_num_extended_sorted_codeword_in_section[x] = in_segment_remain_num_cw as u16;
            si.p_extended_sorted_codebook[x] = rd(&si.p_sorted_codebook, srt_sec_cnt as usize);

            srt_sec_cnt += 1;
            remain_num_cw_in_sort_sec =
                rd(&si.p_num_sorted_codeword_in_section, srt_sec_cnt as usize) as u32;
            in_segment_remain_num_cw = num_segment;
            /* data of a sorted section was integrated in extended sorted section */
        } else {
            /* inSegmentRemainNumCW > remainNumCwInSortSec */
            si.p_num_extended_sorted_codeword_in_section[x] = remain_num_cw_in_sort_sec as u16;
            si.p_extended_sorted_codebook[x] = rd(&si.p_sorted_codebook, srt_sec_cnt as usize);

            in_segment_remain_num_cw -= remain_num_cw_in_sort_sec;
            srt_sec_cnt += 1;
            remain_num_cw_in_sort_sec =
                rd(&si.p_num_sorted_codeword_in_section, srt_sec_cnt as usize) as u32;
            /* data of a sorted section was integrated in extended sorted section */
        }
        si.p_max_len_of_cb_in_ext_srt_sec[x] = f_min(
            rd(&A_MAX_CW_LEN, si.p_extended_sorted_codebook[x] as usize) as i32,
            length_of_longest_codeword as i32,
        ) as u8;

        x_srt_sc_cnt += 1;

        if x_srt_sc_cnt >= (MAX_SFB_HCR + MAX_HCR_SETS) as u32 {
            p_hcr.dec_in_out.error_log |= EXTENDED_SORTED_COUNTER_OVERFLOW;
            return;
        }
    }
    si.p_num_extended_sorted_codeword_in_section[x_srt_sc_cnt as usize] = 0;
}

/// `DeriveNumberOfExtendedSortedSectionsInSets(numSegment, pNumExtendedSortedCodewordInSection,
/// numExtendedSortedCodewordInSectionIdx, pNumExtendedSortedSectionsInSets,
/// numExtendedSortedSectionsInSetsIdx)`: calculates the number of extended sorted sections which
/// belong to the sets (set 0 holds the PCWs).
pub fn derive_number_of_extended_sorted_sections_in_sets(
    num_segment: u32,
    p_num_extended_sorted_codeword_in_section: &[u16],
    mut num_extended_sorted_codeword_in_section_idx: i32,
    p_num_extended_sorted_sections_in_sets: &mut [u16],
    mut num_extended_sorted_sections_in_sets_idx: i32,
) {
    let mut counter: u16 = 0;
    let mut cw_sum: u32 = 0;
    let p_num_ex_sort_cw_in_sec = p_num_extended_sorted_codeword_in_section;
    let p_num_ex_sort_sec_in_sets = p_num_extended_sorted_sections_in_sets;

    while rd(p_num_ex_sort_cw_in_sec, num_extended_sorted_codeword_in_section_idx as usize) != 0 {
        cw_sum = cw_sum.wrapping_add(rd(
            p_num_ex_sort_cw_in_sec,
            num_extended_sorted_codeword_in_section_idx as usize,
        ) as u32);
        num_extended_sorted_codeword_in_section_idx += 1;
        if num_extended_sorted_codeword_in_section_idx >= MAX_SFB_HCR + MAX_HCR_SETS {
            return;
        }
        if cw_sum > num_segment {
            return;
        }
        counter = counter.wrapping_add(1);
        if counter as i32 > 1024 / 4 {
            return;
        }
        if cw_sum == num_segment {
            wr(
                p_num_ex_sort_sec_in_sets,
                num_extended_sorted_sections_in_sets_idx as usize,
                counter,
            );
            num_extended_sorted_sections_in_sets_idx += 1;
            if num_extended_sorted_sections_in_sets_idx >= MAX_HCR_SETS {
                return;
            }
            counter = 0;
            cw_sum = 0;
        }
    }
    /* save last entry for the last - probably shorter - set */
    wr(p_num_ex_sort_sec_in_sets, num_extended_sorted_sections_in_sets_idx as usize, counter);
}

/// `DecodePCWs(bs, pHcr)`: decodes all priority codewords (PCWs) of a spectrum (set 0).
///
/// `spec` is the spectrum of the channel (`decInOut.pQuantizedSpectralCoefficientsBase`).
pub fn decode_pcws(bs: &mut FdkBitStream, p_hcr: &mut CErHcrInfo, spec_buf: &mut [FixpDbl]) {
    let mut num_extended_sorted_codeword_in_section_idx =
        p_hcr.section_info.num_extended_sorted_codeword_in_section_idx;
    let mut extended_sorted_codebook_idx = p_hcr.section_info.extended_sorted_codebook_idx;
    let mut num_extended_sorted_sections_in_sets_idx =
        p_hcr.section_info.num_extended_sorted_sections_in_sets_idx;
    let p_quantized_spectral_coefficients = spec_long();
    let mut quantized_spectral_coefficients_idx: i32 =
        p_hcr.dec_in_out.quantized_spectral_coefficients_idx;
    // index of pLeftStartOfSegment and pRemainingBitsInSegment
    let mut seg = 0usize;
    let mut max_len_of_cb_in_ext_srt_sec_idx = p_hcr.section_info.max_len_of_cb_in_ext_srt_sec_idx;
    let bitstream_anchor = p_hcr.dec_in_out.bitstream_anchor;
    let mut num_decoded_bits: i32;

    /* clear result array */
    {
        let start = (p_quantized_spectral_coefficients as isize
            + quantized_spectral_coefficients_idx as isize)
            .max(0) as usize;
        let end = (start + 1024).min(spec_buf.len());
        if start < end {
            spec_buf[start..end].fill(0);
        }
    }

    /* decode all PCWs in the extended sorted section(s) belonging to set 0 */
    let mut ext_sort_sec: u16 = rd(
        &p_hcr.section_info.p_num_extended_sorted_sections_in_sets,
        num_extended_sorted_sections_in_sets_idx as usize,
    );
    while ext_sort_sec != 0 {
        /* get codebook for this extended sorted section and increment ptr to cb of next ext.
        sort sec */
        let codebook: u8 = rd(
            &p_hcr.section_info.p_extended_sorted_codebook,
            extended_sorted_codebook_idx as usize,
        );
        extended_sorted_codebook_idx += 1;
        if extended_sorted_codebook_idx >= MAX_SFB_HCR + MAX_HCR_SETS {
            return;
        }
        // The extended sorted codebooks are codebooks (or priorities) below 32.
        let cbi = (codebook as usize) & (MAX_CB as usize - 1);
        /* get dimension of codebook of this extended sort. sec. */
        let dimension: u8 = A_DIM_CB[cbi];
        let p_current_tree: &[u32] = A_HUFF_TABLE[cbi]; /* convert codebook to pointer to QSCs */
        let p_quant_val_base: &[i8] = A_QUANT_TABLE[cbi]; /* convert codebook to index to QSCs */
        let max_allowed_cw_len: u8 = rd(
            &p_hcr.section_info.p_max_len_of_cb_in_ext_srt_sec,
            max_len_of_cb_in_ext_srt_sec_idx as usize,
        );
        max_len_of_cb_in_ext_srt_sec_idx += 1;
        if max_len_of_cb_in_ext_srt_sec_idx >= MAX_SFB_HCR + MAX_HCR_SETS {
            return;
        }

        /* switch for decoding with different codebooks: */
        if A_SIGN_CB[cbi] == 0 {
            /* no sign bits follow after the codeword-body */
            /* PCW_BodyONLY */
            /*==============*/

            let mut cur_ext_sort_cw_in_sec: u16 = rd(
                &p_hcr.section_info.p_num_extended_sorted_codeword_in_section,
                num_extended_sorted_codeword_in_section_idx as usize,
            );
            while cur_ext_sort_cw_in_sec != 0 {
                num_decoded_bits = 0;

                /* decode PCW_BODY */
                let q = {
                    let sg = &mut p_hcr.segment_info;
                    let mut d_left = 0;
                    let mut d_rem = 0;
                    let (left, rem) = seg_refs(sg, seg, &mut d_left, &mut d_rem);
                    decode_pcw_body(
                        bs,
                        bitstream_anchor,
                        p_current_tree,
                        left,
                        rem,
                        &mut num_decoded_bits,
                    )
                };

                /* result is written out here because NO sign bits follow the body */
                for k in 0..dimension as usize {
                    /* write quant. spec. coef. into spectrum; sign is already valid */
                    wr(
                        spec_buf,
                        p_quantized_spectral_coefficients
                            + quantized_spectral_coefficients_idx as usize,
                        p_quant_val_base[q + k] as FixpDbl,
                    );
                    quantized_spectral_coefficients_idx += 1;
                    if quantized_spectral_coefficients_idx >= 1024 {
                        return;
                    }
                }

                /* one more PCW should be decoded */

                if (max_allowed_cw_len as i32) < (num_decoded_bits + ERROR_PCW_BODY_ONLY_TOO_LONG) {
                    p_hcr.dec_in_out.error_log |= TOO_MANY_PCW_BODY_BITS_DECODED;
                }

                let rem = rd(&p_hcr.segment_info.p_remaining_bits_in_segment, seg);
                if 1 == err_detect_pcw_segmentation(
                    (rem as i32 - ERROR_PCW_BODY) as i8,
                    p_hcr,
                    PCW_BODY,
                    spec_buf,
                    quantized_spectral_coefficients_idx as isize - dimension as isize,
                    dimension,
                ) {
                    return;
                }
                seg += 1; /* update pointer for decoding the next PCW */
                cur_ext_sort_cw_in_sec -= 1;
            }
        } else if (codebook < 11) && (A_SIGN_CB[cbi] == 1) {
            /* possibly there follow 1,2,3 or 4 sign bits after the codeword-body */
            /* PCW_Body and PCW_Sign */
            /*=======================*/

            let mut cur_ext_sort_cw_in_sec: u16 = rd(
                &p_hcr.section_info.p_num_extended_sorted_codeword_in_section,
                num_extended_sorted_codeword_in_section_idx as usize,
            );
            while cur_ext_sort_cw_in_sec != 0 {
                num_decoded_bits = 0;

                let err = {
                    let sg = &mut p_hcr.segment_info;
                    let mut d_left = 0;
                    let mut d_rem = 0;
                    let (left, rem) = seg_refs(sg, seg, &mut d_left, &mut d_rem);
                    let q = decode_pcw_body(
                        bs,
                        bitstream_anchor,
                        p_current_tree,
                        left,
                        rem,
                        &mut num_decoded_bits,
                    );

                    decode_pcw_sign(
                        bs,
                        bitstream_anchor,
                        dimension as u32,
                        &p_quant_val_base[q..],
                        spec_buf,
                        &mut quantized_spectral_coefficients_idx,
                        left,
                        rem,
                        &mut num_decoded_bits,
                    )
                };
                if err != 0 {
                    return;
                }
                /* one more PCW should be decoded */

                if (max_allowed_cw_len as i32) < (num_decoded_bits + ERROR_PCW_BODY_SIGN_TOO_LONG) {
                    p_hcr.dec_in_out.error_log |= TOO_MANY_PCW_BODY_SIGN_BITS_DECODED;
                }

                let rem = rd(&p_hcr.segment_info.p_remaining_bits_in_segment, seg);
                if 1 == err_detect_pcw_segmentation(
                    (rem as i32 - ERROR_PCW_BODY_SIGN) as i8,
                    p_hcr,
                    PCW_BODY_SIGN,
                    spec_buf,
                    quantized_spectral_coefficients_idx as isize - dimension as isize,
                    dimension,
                ) {
                    return;
                }
                seg += 1;
                cur_ext_sort_cw_in_sec -= 1;
            }
        } else if (A_SIGN_CB[cbi] == 1) && (codebook >= 11) {
            /* possibly there follow some sign bits and maybe one or two escape sequences after
            the cw-body */
            /* PCW_Body, PCW_Sign and maybe PCW_Escape */
            /*=========================================*/

            let mut cur_ext_sort_cw_in_sec: u16 = rd(
                &p_hcr.section_info.p_num_extended_sorted_codeword_in_section,
                num_extended_sorted_codeword_in_section_idx as usize,
            );
            while cur_ext_sort_cw_in_sec != 0 {
                num_decoded_bits = 0;

                {
                    let error_log = &mut p_hcr.dec_in_out.error_log;
                    let sg = &mut p_hcr.segment_info;
                    let mut d_left = 0;
                    let mut d_rem = 0;
                    let (left, rem) = seg_refs(sg, seg, &mut d_left, &mut d_rem);

                    /* decode PCW_BODY */
                    let q = decode_pcw_body(
                        bs,
                        bitstream_anchor,
                        p_current_tree,
                        left,
                        rem,
                        &mut num_decoded_bits,
                    );

                    let err = decode_pcw_sign(
                        bs,
                        bitstream_anchor,
                        dimension as u32,
                        &p_quant_val_base[q..],
                        spec_buf,
                        &mut quantized_spectral_coefficients_idx,
                        left,
                        rem,
                        &mut num_decoded_bits,
                    );
                    if err != 0 {
                        return;
                    }

                    /* decode PCW_ESCAPE if present */
                    quantized_spectral_coefficients_idx -= DIMENSION_OF_ESCAPE_CODEBOOK;

                    for _ in 0..2 {
                        let pos = p_quantized_spectral_coefficients
                            .wrapping_add(quantized_spectral_coefficients_idx as usize);
                        let v = rd(spec_buf, pos);
                        if fixp_abs(v) == ESCAPE_VALUE as FixpDbl {
                            let e = decode_escape_sequence(
                                bs,
                                bitstream_anchor,
                                v,
                                left,
                                rem,
                                &mut num_decoded_bits,
                                error_log,
                            );
                            wr(spec_buf, pos, e as FixpDbl);
                        }
                        quantized_spectral_coefficients_idx += 1;
                        if quantized_spectral_coefficients_idx >= 1024 {
                            return;
                        }
                    }
                }

                /* one more PCW should be decoded */

                if (max_allowed_cw_len as i32)
                    < (num_decoded_bits + ERROR_PCW_BODY_SIGN_ESC_TOO_LONG)
                {
                    p_hcr.dec_in_out.error_log |= TOO_MANY_PCW_BODY_SIGN_ESC_BITS_DECODED;
                }

                let rem = rd(&p_hcr.segment_info.p_remaining_bits_in_segment, seg);
                if 1 == err_detect_pcw_segmentation(
                    (rem as i32 - ERROR_PCW_BODY_SIGN_ESC) as i8,
                    p_hcr,
                    PCW_BODY_SIGN_ESC,
                    spec_buf,
                    quantized_spectral_coefficients_idx as isize
                        - DIMENSION_OF_ESCAPE_CODEBOOK as isize,
                    DIMENSION_OF_ESCAPE_CODEBOOK as u8,
                ) {
                    return;
                }
                seg += 1;
                cur_ext_sort_cw_in_sec -= 1;
            }
        }

        /* all PCWs belonging to this extended section should be decoded */
        num_extended_sorted_codeword_in_section_idx += 1;
        if num_extended_sorted_codeword_in_section_idx >= MAX_SFB_HCR + MAX_HCR_SETS {
            return;
        }
        ext_sort_sec -= 1;
    }
    /* all PCWs should be decoded */

    num_extended_sorted_sections_in_sets_idx += 1;
    if num_extended_sorted_sections_in_sets_idx >= MAX_HCR_SETS {
        return;
    }

    /* Write back indexes into structure */
    p_hcr.section_info.num_extended_sorted_codeword_in_section_idx =
        num_extended_sorted_codeword_in_section_idx;
    p_hcr.section_info.extended_sorted_codebook_idx = extended_sorted_codebook_idx;
    p_hcr.section_info.num_extended_sorted_sections_in_sets_idx =
        num_extended_sorted_sections_in_sets_idx;
    p_hcr.dec_in_out.quantized_spectral_coefficients_idx = quantized_spectral_coefficients_idx;
    p_hcr.section_info.max_len_of_cb_in_ext_srt_sec_idx = max_len_of_cb_in_ext_srt_sec_idx;
}

/// `&pLeftStartOfSegment[seg]` and `&pRemainingBitsInSegment[seg]`; a segment index outside the
/// arrays (at most `numSegment <= 512` PCWs are decoded) uses the dummies.
#[inline]
fn seg_refs<'a>(
    sg: &'a mut HcrSegmentInfo,
    seg: usize,
    d_left: &'a mut i32,
    d_rem: &'a mut i8,
) -> (&'a mut i32, &'a mut i8) {
    let left = match sg.p_left_start_of_segment.get_mut(seg) {
        Some(x) => x,
        None => d_left,
    };
    let rem = match sg.p_remaining_bits_in_segment.get_mut(seg) {
        Some(x) => x,
        None => d_rem,
    };
    (left, rem)
}

/// `errDetectPcwSegmentation(remainingBitsInSegment, pHcr, kind, qsc_base_of_cw, dimension)`:
/// checks immediately after every decoded PCW whether too many bits have been read from the
/// current segment. If so, the two or four quantized spectral coefficients of the current
/// codeword are marked (for being detected by the concealment later).
///
/// `qsc_base_of_cw` is the offset of the codeword's first line in `spec`.
pub fn err_detect_pcw_segmentation(
    remaining_bits_in_segment: i8,
    p_hcr: &mut CErHcrInfo,
    kind: PcwType,
    spec_buf: &mut [FixpDbl],
    qsc_base_of_cw: isize,
    dimension: u8,
) -> u8 {
    if remaining_bits_in_segment < 0 {
        /* log the error */
        match kind {
            PCW_BODY => p_hcr.dec_in_out.error_log |= SEGMENT_OVERRIDE_ERR_PCW_BODY,
            PCW_BODY_SIGN => p_hcr.dec_in_out.error_log |= SEGMENT_OVERRIDE_ERR_PCW_BODY_SIGN,
            PCW_BODY_SIGN_ESC => {
                p_hcr.dec_in_out.error_log |= SEGMENT_OVERRIDE_ERR_PCW_BODY_SIGN_ESC
            }
            _ => {}
        }
        /* mark the erred lines */
        let mut q = qsc_base_of_cw;
        for _ in 0..dimension {
            wr(spec_buf, q as usize, Q_VALUE_INVALID as FixpDbl);
            q += 1;
        }
        return 1;
    }
    0
}

/// `errDetectWithinSegmentationFinal(pHcr)`: checks if all segments are empty after decoding.
/// No lines are marked as invalid, because it cannot be traced back where the remaining bits
/// came from.
pub fn err_detect_within_segmentation_final(p_hcr: &mut CErHcrInfo) {
    let mut segmentation_error_flag: u8 = 0;
    let num_segment = p_hcr.segment_info.num_segment;

    let mut i: u16 = num_segment as u16;
    let mut idx = 0usize;
    while i != 0 {
        if rd(&p_hcr.segment_info.p_remaining_bits_in_segment, idx) != 0 {
            segmentation_error_flag = 1;
        }
        idx += 1;
        i -= 1;
    }
    if segmentation_error_flag == 1 {
        p_hcr.dec_in_out.error_log |= BIT_IN_SEGMENTATION_ERROR;
    }
}

/// `CarryBitToBranchValue(carryBit, treeNode, branchValue, branchNode)`: walks one step within
/// the decoding tree. Which branch is taken depends on `carryBit`.
#[inline]
pub fn carry_bit_to_branch_value(
    carry_bit: u8,
    tree_node: u32,
    branch_value: &mut u32,
    branch_node: &mut u32,
) {
    if carry_bit == 0 {
        *branch_node = (tree_node & MASK_LEFT as u32) >> LEFT_OFFSET; /* MASK_LEFT:  00FFF000 */
    } else {
        *branch_node = tree_node & MASK_RIGHT as u32; /* MASK_RIGHT: 00000FFF */
    }

    *branch_value = *branch_node & CLR_BIT_10 as u32; /* clear bit 10 (if set) */
}

/// `DecodePCW_Body(bs, bsAnchor, pCurrentTree, pQuantValBase, pLeftStartOfSegment,
/// pRemainingBitsInSegment, pNumDecodedBits)`: decodes the body of a priority codeword (PCW).
///
/// Returns the offset in `pQuantValBase` of the first of the two or four quantized spectral
/// coefficients (C returns the pointer `pQuantValBase + branchValue`).
pub fn decode_pcw_body(
    bs: &mut FdkBitStream,
    bs_anchor: i32,
    p_current_tree: &[u32],
    p_left_start_of_segment: &mut i32,
    p_remaining_bits_in_segment: &mut i8,
    p_num_decoded_bits: &mut i32,
) -> usize {
    let mut branch_node: u32 = 0;
    let mut branch_value: u32 = 0;

    /* decode PCW_BODY */
    /* get first node of current tree belonging to current codebook */
    let mut tree_node = p_current_tree[0];

    /* decode whole PCW-codeword-body */
    loop {
        let carry_bit =
            hcr_get_a_bit_from_bitstream_ltr(bs, bs_anchor, p_left_start_of_segment) as u8;
        *p_remaining_bits_in_segment = p_remaining_bits_in_segment.wrapping_sub(1);
        *p_num_decoded_bits += 1;

        carry_bit_to_branch_value(carry_bit, tree_node, &mut branch_value, &mut branch_node);

        if (branch_node & TEST_BIT_10 as u32) == TEST_BIT_10 as u32 {
            /* test bit 10 ; if set --> codeword-body is complete */
            break; /* end of branch in tree reached  i.e. a whole PCW-Body is decoded */
        } else {
            /* update treeNode for further step in decoding tree; every branch of the trees
            stays inside its tree (checked by the differential tests) */
            tree_node = p_current_tree[branch_value as usize];
        }
    }

    /* update pointer to valid first of 2 or 4 quantized values */
    branch_value as usize
}

/// `DecodeEscapeSequence(bs, bsAnchor, quantSpecCoef, pLeftStartOfSegment,
/// pRemainingBitsInSegment, pNumDecodedBits, errorWord)`: decodes one escape sequence (escape
/// prefix, then escape word) for a quantized spectral value of absolute value 16 of an escape
/// codebook.
pub fn decode_escape_sequence(
    bs: &mut FdkBitStream,
    bs_anchor: i32,
    mut quant_spec_coef: i32,
    p_left_start_of_segment: &mut i32,
    p_remaining_bits_in_segment: &mut i8,
    p_num_decoded_bits: &mut i32,
    error_word: &mut u32,
) -> i32 {
    let mut escape_ones_counter: u32 = 0;
    let mut escape_word: i32 = 0;

    /* decode escape prefix */
    loop {
        let carry_bit = hcr_get_a_bit_from_bitstream_ltr(bs, bs_anchor, p_left_start_of_segment);
        *p_remaining_bits_in_segment = p_remaining_bits_in_segment.wrapping_sub(1);
        *p_num_decoded_bits += 1;
        if *p_remaining_bits_in_segment < 0 {
            return Q_VALUE_INVALID;
        }

        if carry_bit != 0 {
            escape_ones_counter += 1;
        } else {
            escape_ones_counter += 4;
            break;
        }
    }

    /* decode escape word */
    let mut i = escape_ones_counter;
    while i != 0 {
        let carry_bit = hcr_get_a_bit_from_bitstream_ltr(bs, bs_anchor, p_left_start_of_segment);
        *p_remaining_bits_in_segment = p_remaining_bits_in_segment.wrapping_sub(1);
        *p_num_decoded_bits += 1;
        if *p_remaining_bits_in_segment < 0 {
            return Q_VALUE_INVALID;
        }

        // escape_word <<= 1 on an INT: two's complement (the remaining bits limit the number of
        // shifts to less than 128, so the value can wrap)
        escape_word = escape_word.wrapping_shl(1);
        escape_word |= carry_bit as i32;
        i -= 1;
    }

    let sign: i32 = if quant_spec_coef >= 0 { 1 } else { -1 };

    if escape_ones_counter < 13 {
        quant_spec_coef = sign * ((1i32 << escape_ones_counter) + escape_word);
    } else {
        *error_word |= TOO_MANY_PCW_BODY_SIGN_ESC_BITS_DECODED;
        quant_spec_coef = Q_VALUE_INVALID;
    }
    quant_spec_coef
}

/// `DecodePCW_Sign(bs, bsAnchor, codebookDim, pQuantVal, pQuantSpecCoef, quantSpecCoefIdx,
/// pLeftStartOfSegment, pRemainingBitsInSegment, pNumDecodedBits)`: decodes the sign bits of a
/// priority codeword (PCW) and writes the resulting quantized spectral values into the unsorted
/// sections. Returns 0, or -1 on an error.
pub fn decode_pcw_sign(
    bs: &mut FdkBitStream,
    bs_anchor: i32,
    codebook_dim: u32,
    p_quant_val: &[i8],
    p_quant_spec_coef: &mut [FixpDbl],
    quant_spec_coef_idx: &mut i32,
    p_left_start_of_segment: &mut i32,
    p_remaining_bits_in_segment: &mut i8,
    p_num_decoded_bits: &mut i32,
) -> i32 {
    for k in 0..codebook_dim as usize {
        let quant_spec_coef = p_quant_val[k] as i32;
        if quant_spec_coef != 0 {
            let carry_bit =
                hcr_get_a_bit_from_bitstream_ltr(bs, bs_anchor, p_left_start_of_segment);
            *p_remaining_bits_in_segment = p_remaining_bits_in_segment.wrapping_sub(1);
            *p_num_decoded_bits += 1;
            if *p_remaining_bits_in_segment < 0 || *p_num_decoded_bits >= (1024 >> 1) {
                return -1;
            }

            /* adapt sign of values according to the decoded sign bit */
            if carry_bit != 0 {
                wr(p_quant_spec_coef, *quant_spec_coef_idx as usize, -quant_spec_coef);
            } else {
                wr(p_quant_spec_coef, *quant_spec_coef_idx as usize, quant_spec_coef);
            }
        } else {
            wr(p_quant_spec_coef, *quant_spec_coef_idx as usize, 0);
        }
        *quant_spec_coef_idx += 1;
        if *quant_spec_coef_idx >= 1024 {
            return -1;
        }
    }
    0
}

/// `HcrMuteErroneousLines(hHcr)`: mutes the spectral lines which have been marked as erroneous
/// (`Q_VALUE_INVALID`).
///
/// `spec` is the spectrum of the channel (`decInOut.pQuantizedSpectralCoefficientsBase`).
pub fn hcr_mute_erroneous_lines(_h_hcr: &CErHcrInfo, spec_buf: &mut [FixpDbl]) {
    let n = spec_buf.len().min(1024);
    /* if there is a line with value Q_VALUE_INVALID mute it */
    for c in &mut spec_buf[spec_long()..n] {
        if *c == Q_VALUE_INVALID as FixpDbl {
            *c = 0; /* muting */
        }
    }
}
