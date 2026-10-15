// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACenc/src/dyn_bits.h, libAACenc/src/dyn_bits.cpp.

//! Port of `dyn_bits.h`: noiseless coding (section data and dynamic bit count).
//!
//! The bit look-up tables live in the shared dynamic RAM in C. `FDKaacEnc_dynBitCount()` only
//! reads entries it wrote in the same call (rows `0..maxSfbPerGroup` of `bitLookUp`, rebuilt for
//! every group; `mergeGainLookUp` at section starts, all written by stage 2 of the same group
//! before they are read), so their initial contents do not matter.

use crate::aac_enc_rom::{FDK_AAC_ENC_SIDE_INFO_TAB_LONG, FDK_AAC_ENC_SIDE_INFO_TAB_SHORT};
use crate::aacenc_pns::NO_NOISE_PNS;
use crate::bit_cnt::{
    fdk_aac_enc_bit_count, fdk_aac_enc_bit_count_scalefactor_delta, CODE_BOOK_ESC_NDX,
    CODE_BOOK_IS_IN_PHASE_NO, CODE_BOOK_IS_OUT_OF_PHASE_NO, CODE_BOOK_PNS_NO, CODE_BOOK_SCF_LAV,
    CODE_BOOK_ZERO_NO, INVALID_BITCOUNT,
};
use crate::psy_const::{FALSE, MAX_GROUPED_SFB, MAX_SFB_LONG, SHORT_WINDOW, TRUE};
use fdk_aac_rs_fdk::audio::AC_ER_VCB11;
use fdk_aac_rs_fdk::common_fix::fix_min;

/// `MAX_SECTIONS`
pub const MAX_SECTIONS: usize = MAX_GROUPED_SFB;
/// `SECT_ESC_VAL_LONG`
pub const SECT_ESC_VAL_LONG: i32 = 31;
/// `SECT_ESC_VAL_SHORT`
pub const SECT_ESC_VAL_SHORT: i32 = 7;
/// `CODE_BOOK_BITS`
pub const CODE_BOOK_BITS: i32 = 4;
/// `SECT_BITS_LONG`
pub const SECT_BITS_LONG: i32 = 5;
/// `SECT_BITS_SHORT`
pub const SECT_BITS_SHORT: i32 = 3;
/// `PNS_PCM_BITS`
pub const PNS_PCM_BITS: i32 = 9;

/// `SECTION_INFO` (layout identical to the C struct).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SectionInfo {
    pub code_book: i32,
    pub sfb_start: i32,
    pub sfb_cnt: i32,
    /// huff + si !
    pub section_bits: i32,
}

/// `SECTION_DATA` (layout identical to the C struct).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SectionData {
    pub block_type: i32,
    pub no_of_groups: i32,
    pub sfb_cnt: i32,
    pub max_sfb_per_group: i32,
    pub sfb_per_group: i32,
    pub no_of_sections: i32,
    pub huffsection: [SectionInfo; MAX_SECTIONS],
    /// sectioning bits
    pub side_info_bits: i32,
    /// huffman coded bits
    pub huffman_bits: i32,
    /// scalefac coded bits
    pub scalefac_bits: i32,
    /// noiseEnergy coded bits
    pub noise_nrg_bits: i32,
    /// first scf to be coded
    pub first_scf: i32,
}

impl Default for SectionData {
    fn default() -> Self {
        SectionData {
            block_type: 0,
            no_of_groups: 0,
            sfb_cnt: 0,
            max_sfb_per_group: 0,
            sfb_per_group: 0,
            no_of_sections: 0,
            huffsection: [SectionInfo::default(); MAX_SECTIONS],
            side_info_bits: 0,
            huffman_bits: 0,
            scalefac_bits: 0,
            noise_nrg_bits: 0,
            first_scf: 0,
        }
    }
}

/// Number of entries per scalefactor band of the bit look-up table
/// (`CODE_BOOK_ESC_NDX + 1`).
pub const BIT_LOOK_UP_WIDTH: usize = 12;
const _: () = assert!(BIT_LOOK_UP_WIDTH == CODE_BOOK_ESC_NDX as usize + 1);

/// `struct BITCNTR_STATE`
///
/// In C, `bitLookUp` and `mergeGainLookUp` point into the shared dynamic RAM of the encoder
/// (`GetRam_aacEnc_BitLookUp`, `GetRam_aacEnc_MergeGainLookUp`); here they are owned buffers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BitcntrState {
    /// `bitLookUp[MAX_SFB_LONG * (CODE_BOOK_ESC_NDX + 1)]`
    pub bit_look_up: Vec<i32>,
    /// `mergeGainLookUp[MAX_SFB_LONG]`
    pub merge_gain_look_up: Vec<i32>,
}

impl Default for BitcntrState {
    fn default() -> Self {
        BitcntrState {
            bit_look_up: vec![0; MAX_SFB_LONG * BIT_LOOK_UP_WIDTH],
            merge_gain_look_up: vec![0; MAX_SFB_LONG],
        }
    }
}

/// Row `i` of the bit look-up table (`bitLookUp[i]`).
#[inline(always)]
fn row(bit_look_up: &[i32], i: i32) -> &[i32; BIT_LOOK_UP_WIDTH] {
    bit_look_up[i as usize * BIT_LOOK_UP_WIDTH..(i as usize + 1) * BIT_LOOK_UP_WIDTH]
        .try_into()
        .unwrap()
}

/// `FDKaacEnc_getSideInfoBits`
#[inline]
fn fdk_aac_enc_get_side_info_bits(
    huffsection: &SectionInfo,
    side_info_tab: &[i16],
    use_hcr: i32,
) -> i32 {
    if use_hcr != 0 && ((huffsection.code_book == 11) || (huffsection.code_book >= 16)) {
        5
    } else {
        side_info_tab[huffsection.sfb_cnt as usize] as i32
    }
}

/// `FDKaacEnc_buildBitLookUp`: count bits using all possible tables.
///
/// `huffsection` is the section array starting at the group's first section.
fn fdk_aac_enc_build_bit_look_up(
    quant_spectrum: &[i16],
    max_sfb: i32,
    sfb_offset: &[i32],
    sfb_max: &[u32],
    bit_look_up: &mut [i32],
    huffsection: &mut [SectionInfo],
) {
    for i in 0..max_sfb as usize {
        huffsection[i].sfb_cnt = 1;
        huffsection[i].sfb_start = i as i32;
        huffsection[i].section_bits = INVALID_BITCOUNT;
        huffsection[i].code_book = -1;
        let sfb_width = sfb_offset[i + 1] - sfb_offset[i];
        fdk_aac_enc_bit_count(
            &quant_spectrum[sfb_offset[i] as usize..],
            sfb_width,
            sfb_max[i] as i32,
            &mut bit_look_up[i * BIT_LOOK_UP_WIDTH..(i + 1) * BIT_LOOK_UP_WIDTH],
        );
    }
}

/// `FDKaacEnc_findBestBook`: essential helper functions
#[inline]
fn fdk_aac_enc_find_best_book(
    bc: &[i32; BIT_LOOK_UP_WIDTH],
    book: &mut i32,
    _use_vcb11: i32,
) -> i32 {
    let mut min_bits = INVALID_BITCOUNT;

    // j = 0..=CODE_BOOK_ESC_NDX
    for (j, &b) in bc.iter().enumerate() {
        if b < min_bits {
            min_bits = b;
            *book = j as i32;
        }
    }
    min_bits
}

/// `FDKaacEnc_findMinMergeBits`
#[inline]
fn fdk_aac_enc_find_min_merge_bits(
    bc1: &[i32; BIT_LOOK_UP_WIDTH],
    bc2: &[i32; BIT_LOOK_UP_WIDTH],
    _use_vcb11: i32,
) -> i32 {
    let mut min_bits = INVALID_BITCOUNT;

    // j = 0..=CODE_BOOK_ESC_NDX (the entries are at most INVALID_BITCOUNT: the sums do not
    // overflow, and the minimum does not depend on the order)
    for j in 0..BIT_LOOK_UP_WIDTH {
        min_bits = fix_min(min_bits, bc1[j] + bc2[j]);
    }
    min_bits
}

/// `FDKaacEnc_mergeBitLookUp`: `bitLookUp[ndx1] += bitLookUp[ndx2]` (limited to
/// `INVALID_BITCOUNT`).
#[inline]
fn fdk_aac_enc_merge_bit_look_up(bit_look_up: &mut [i32], ndx1: i32, ndx2: i32) {
    let a = ndx1 as usize * BIT_LOOK_UP_WIDTH;
    // (ndx1 == ndx2 doubles the row in C as well: every entry is read before it is written)
    let src: [i32; BIT_LOOK_UP_WIDTH] = *row(bit_look_up, ndx2);
    let dst: &mut [i32; BIT_LOOK_UP_WIDTH] =
        (&mut bit_look_up[a..a + BIT_LOOK_UP_WIDTH]).try_into().unwrap();
    fdk_aac_rs_fdk::fdk_assert!(INVALID_BITCOUNT == 0x1FFFFFFF);
    // j = 0..=CODE_BOOK_ESC_NDX
    for j in 0..BIT_LOOK_UP_WIDTH {
        dst[j] = fix_min(dst[j] + src[j], INVALID_BITCOUNT);
    }
}

/// `FDKaacEnc_findMaxMerge`
#[inline]
fn fdk_aac_enc_find_max_merge(
    merge_gain_look_up: &[i32],
    huffsection: &[SectionInfo],
    max_sfb: i32,
    max_ndx: &mut i32,
) -> i32 {
    let mut max_merge_gain = 0;
    let mut last_max_ndx = 0;

    let mut i = 0;
    while i + huffsection[i as usize].sfb_cnt < max_sfb {
        if merge_gain_look_up[i as usize] > max_merge_gain {
            max_merge_gain = merge_gain_look_up[i as usize];
            last_max_ndx = i;
        }
        i += huffsection[i as usize].sfb_cnt;
    }
    *max_ndx = last_max_ndx;
    max_merge_gain
}

/// `FDKaacEnc_CalcMergeGain`
#[inline]
fn fdk_aac_enc_calc_merge_gain(
    huffsection: &[SectionInfo],
    bit_look_up: &[i32],
    side_info_tab: &[i16],
    ndx1: i32,
    ndx2: i32,
    use_vcb11: i32,
) -> i32 {
    let (h1, h2) = (&huffsection[ndx1 as usize], &huffsection[ndx2 as usize]);
    let merge_bits = side_info_tab[(h1.sfb_cnt + h2.sfb_cnt) as usize] as i32
        + fdk_aac_enc_find_min_merge_bits(
            row(bit_look_up, ndx1),
            row(bit_look_up, ndx2),
            use_vcb11,
        );
    let split_bits = h1.section_bits + h2.section_bits; // Bit amount for splitted huffsections
    let mut merge_gain = split_bits - merge_bits;

    if (h1.code_book == CODE_BOOK_PNS_NO)
        || (h2.code_book == CODE_BOOK_PNS_NO)
        || (h1.code_book == CODE_BOOK_IS_OUT_OF_PHASE_NO)
        || (h2.code_book == CODE_BOOK_IS_OUT_OF_PHASE_NO)
        || (h1.code_book == CODE_BOOK_IS_IN_PHASE_NO)
        || (h2.code_book == CODE_BOOK_IS_IN_PHASE_NO)
    {
        merge_gain = -1;
    }

    merge_gain
}

/// `FDKaacEnc_gmStage0`: sectioning Stage 0: find minimum codbooks
fn fdk_aac_enc_gm_stage0(
    huffsection: &mut [SectionInfo],
    bit_look_up: &[i32],
    max_sfb: i32,
    noise_nrg: &[i32],
    is_book: &[i32],
) {
    for i in 0..max_sfb as usize {
        // Side-Info bits will be calculated in Stage 1!
        if huffsection[i].section_bits == INVALID_BITCOUNT {
            // intensity and pns codebooks are already allocated in bitcount.c
            if noise_nrg[i] != NO_NOISE_PNS {
                huffsection[i].code_book = CODE_BOOK_PNS_NO;
                huffsection[i].section_bits = 0;
            } else if is_book[i] != 0 {
                huffsection[i].code_book = is_book[i];
                huffsection[i].section_bits = 0;
            } else {
                huffsection[i].section_bits = fdk_aac_enc_find_best_book(
                    row(bit_look_up, i as i32),
                    &mut huffsection[i].code_book,
                    0,
                ); // useVCB11 must be 0!!!
            }
        }
    }
}

/// `FDKaacEnc_gmStage1`: sectioning Stage 1: merge all connected regions with the same code
/// book and calculate side info
fn fdk_aac_enc_gm_stage1(
    huffsection: &mut [SectionInfo],
    bit_look_up: &mut [i32],
    max_sfb: i32,
    side_info_tab: &[i16],
    use_vcb11: i32,
) {
    let mut merge_start: i32 = 0;

    loop {
        let mut merge_end = merge_start + 1;
        while merge_end < max_sfb {
            if huffsection[merge_start as usize].code_book
                != huffsection[merge_end as usize].code_book
            {
                break;
            }

            // we can merge. update tables, side info bits will be updated outside of this loop
            huffsection[merge_start as usize].sfb_cnt += 1;
            huffsection[merge_start as usize].section_bits +=
                huffsection[merge_end as usize].section_bits;

            // update bit look up for all code books
            fdk_aac_enc_merge_bit_look_up(bit_look_up, merge_start, merge_end);
            merge_end += 1;
        }

        // add side info info bits
        huffsection[merge_start as usize].section_bits += fdk_aac_enc_get_side_info_bits(
            &huffsection[merge_start as usize],
            side_info_tab,
            use_vcb11,
        );
        huffsection[(merge_end - 1) as usize].sfb_start =
            huffsection[merge_start as usize].sfb_start; // speed up prev search

        merge_start = merge_end;

        if merge_start >= max_sfb {
            break;
        }
    }
}

/// `FDKaacEnc_gmStage2`: sectioning Stage 2: greedy merge algorithm, merge connected sections
/// with maximum bit gain until no more gain is possible
fn fdk_aac_enc_gm_stage2(
    huffsection: &mut [SectionInfo],
    merge_gain_look_up: &mut [i32],
    bit_look_up: &mut [i32],
    max_sfb: i32,
    side_info_tab: &[i16],
    use_vcb11: i32,
) {
    let mut i = 0;
    while i + huffsection[i as usize].sfb_cnt < max_sfb {
        merge_gain_look_up[i as usize] = fdk_aac_enc_calc_merge_gain(
            huffsection,
            bit_look_up,
            side_info_tab,
            i,
            i + huffsection[i as usize].sfb_cnt,
            use_vcb11,
        );
        i += huffsection[i as usize].sfb_cnt;
    }

    loop {
        let mut max_ndx = 0;

        let max_merge_gain =
            fdk_aac_enc_find_max_merge(merge_gain_look_up, huffsection, max_sfb, &mut max_ndx);

        // exit while loop if no more gain is possible
        if max_merge_gain <= 0 {
            break;
        }

        let mut max_ndx_next = max_ndx + huffsection[max_ndx as usize].sfb_cnt;

        // merge sections with maximum bit gain
        huffsection[max_ndx as usize].sfb_cnt += huffsection[max_ndx_next as usize].sfb_cnt;
        huffsection[max_ndx as usize].section_bits +=
            huffsection[max_ndx_next as usize].section_bits - max_merge_gain;

        // update bit look up table for merged huffsection
        fdk_aac_enc_merge_bit_look_up(bit_look_up, max_ndx, max_ndx_next);

        // update mergeLookUpTable
        if max_ndx != 0 {
            let max_ndx_last = huffsection[(max_ndx - 1) as usize].sfb_start;
            merge_gain_look_up[max_ndx_last as usize] = fdk_aac_enc_calc_merge_gain(
                huffsection,
                bit_look_up,
                side_info_tab,
                max_ndx_last,
                max_ndx,
                use_vcb11,
            );
        }
        max_ndx_next = max_ndx + huffsection[max_ndx as usize].sfb_cnt;

        huffsection[(max_ndx_next - 1) as usize].sfb_start =
            huffsection[max_ndx as usize].sfb_start;

        if max_ndx_next < max_sfb {
            merge_gain_look_up[max_ndx as usize] = fdk_aac_enc_calc_merge_gain(
                huffsection,
                bit_look_up,
                side_info_tab,
                max_ndx,
                max_ndx_next,
                use_vcb11,
            );
        }
    }
}

/// `FDKaacEnc_noiselessCounter`: count bits used by the noiseless coder
#[allow(clippy::too_many_arguments)]
fn fdk_aac_enc_noiseless_counter(
    section_data: &mut SectionData,
    merge_gain_look_up: &mut [i32],
    bit_look_up: &mut [i32],
    quant_spectrum: &[i16],
    max_value_in_sfb: &[u32],
    sfb_offset: &[i32],
    block_type: i32,
    noise_nrg: &[i32],
    is_book: &[i32],
    use_vcb11: i32,
) {
    // use appropriate side info table
    let side_info_tab: &[i16] = match block_type {
        SHORT_WINDOW => &FDK_AAC_ENC_SIDE_INFO_TAB_SHORT,
        _ => &FDK_AAC_ENC_SIDE_INFO_TAB_LONG,
    };

    section_data.no_of_sections = 0;
    section_data.huffman_bits = 0;
    section_data.side_info_bits = 0;

    if section_data.max_sfb_per_group == 0 {
        return;
    }

    // loop trough groups
    let mut grp_ndx = 0;
    while grp_ndx < section_data.sfb_cnt {
        // huffsection = sectionData->huffsection + sectionData->noOfSections
        let base = section_data.no_of_sections as usize;
        let max_sfb = section_data.max_sfb_per_group;
        let g = grp_ndx as usize;

        {
            let huffsection = &mut section_data.huffsection[base..];

            // count bits in this group
            fdk_aac_enc_build_bit_look_up(
                quant_spectrum,
                max_sfb,
                &sfb_offset[g..],
                &max_value_in_sfb[g..],
                bit_look_up,
                huffsection,
            );

            // 0.Stage :Find minimum Codebooks
            fdk_aac_enc_gm_stage0(
                huffsection,
                bit_look_up,
                max_sfb,
                &noise_nrg[g..],
                &is_book[g..],
            );

            // 1.Stage :Merge all connected regions with the same code book
            fdk_aac_enc_gm_stage1(huffsection, bit_look_up, max_sfb, side_info_tab, use_vcb11);

            // 2.Stage
            // greedy merge algorithm, merge connected huffsections with maximum bit gain until no
            // more gain is possible
            fdk_aac_enc_gm_stage2(
                huffsection,
                merge_gain_look_up,
                bit_look_up,
                max_sfb,
                side_info_tab,
                use_vcb11,
            );
        }

        // compress output, calculate total huff and side bits
        // since we did not update the actual codebook in stage 2 to save time, we must set it
        // here for later use in bitenc
        let mut i = 0;
        while i < max_sfb {
            let h = base + i as usize;
            let cb = section_data.huffsection[h].code_book;
            if (cb == CODE_BOOK_PNS_NO)
                || (cb == CODE_BOOK_IS_OUT_OF_PHASE_NO)
                || (cb == CODE_BOOK_IS_IN_PHASE_NO)
            {
                section_data.huffsection[h].section_bits = 0;
            } else {
                // the sections in the sectionData are now marked with the optimal code book
                let mut book = section_data.huffsection[h].code_book;
                fdk_aac_enc_find_best_book(row(bit_look_up, i), &mut book, use_vcb11);
                section_data.huffsection[h].code_book = book;

                section_data.huffman_bits += section_data.huffsection[h].section_bits
                    - fdk_aac_enc_get_side_info_bits(
                        &section_data.huffsection[h],
                        side_info_tab,
                        use_vcb11,
                    );
            }

            section_data.huffsection[h].sfb_start += grp_ndx;

            // sum up side info bits (section data bits)
            section_data.side_info_bits += fdk_aac_enc_get_side_info_bits(
                &section_data.huffsection[h],
                side_info_tab,
                use_vcb11,
            );
            let s = section_data.huffsection[h];
            section_data.huffsection[section_data.no_of_sections as usize] = s;
            section_data.no_of_sections += 1;
            i += s.sfb_cnt;
        }
        grp_ndx += section_data.sfb_per_group;
    }
}

/// `FDKaacEnc_scfCount`: count bits used by scalefactors.
///
/// not in all cases if maxValueInSfb[] == 0 we set deltaScf to zero. only if the difference of
/// the last and future scalefacGain is not greater then CODE_BOOK_SCF_LAV (60).
///
/// example:
/// ```text
///              ^
/// scalefacGain |
///              |
///              |       last 75
///              |          |
///              |          |
///              |          |
///              |          |      current 50
///              |          |          |
///              |          |          |
///              |          |          |
///              |          |          |
///              |          |          |      future 5
///              |          |          |          |
///              --- ... ---------------------------- ... --------->
///                                                            sfb
/// ```
///
/// if maxValueInSfb[] of current is zero because of a notfallstrategie, we do not save bits and
/// transmit a deltaScf of 25. otherwise the deltaScf between the last scalfacGain (75) and the
/// future scalefacGain (5) is 70.
fn fdk_aac_enc_scf_count(
    scalefac_gain: Option<&[i32]>,
    max_value_in_sfb: &[u32],
    section_data: &mut SectionData,
    is_scale: &[i32],
) {
    let mut last_val_scf: i32 = 0;
    let mut delta_scf: i32 = 0;
    let mut found;
    let mut scf_skip_counter: i32 = 0;
    let mut last_val_is: i32 = 0;

    section_data.scalefac_bits = 0;

    let Some(scalefac_gain) = scalefac_gain else { return };

    section_data.first_scf = 0;

    let n_sec = section_data.no_of_sections as usize;
    for i in 0..n_sec {
        if section_data.huffsection[i].code_book != CODE_BOOK_ZERO_NO {
            section_data.first_scf = section_data.huffsection[i].sfb_start;
            last_val_scf = scalefac_gain[section_data.first_scf as usize];
            break;
        }
    }

    for i in 0..n_sec {
        let hs = section_data.huffsection[i];
        if (hs.code_book == CODE_BOOK_IS_OUT_OF_PHASE_NO)
            || (hs.code_book == CODE_BOOK_IS_IN_PHASE_NO)
        {
            for j in hs.sfb_start..hs.sfb_start + hs.sfb_cnt {
                let delta_is = is_scale[j as usize].wrapping_sub(last_val_is);
                last_val_is = is_scale[j as usize];
                section_data.scalefac_bits += fdk_aac_enc_bit_count_scalefactor_delta(delta_is);
            }
        }
        // Intensity
        else if (hs.code_book != CODE_BOOK_ZERO_NO) && (hs.code_book != CODE_BOOK_PNS_NO) {
            let tmp = hs.sfb_start + hs.sfb_cnt;
            for j in hs.sfb_start..tmp {
                // check if we can repeat the last value to save bits
                if max_value_in_sfb[j as usize] == 0 {
                    found = 0;
                    // are scalefactors skipped?
                    if scf_skip_counter == 0 {
                        // end of section
                        if j == (tmp - 1) {
                            found = 0; // search in other sections for maxValueInSfb != 0
                        } else {
                            // search in this section for the next maxValueInSfb[] != 0
                            for k in (j + 1)..tmp {
                                if max_value_in_sfb[k as usize] != 0 {
                                    found = 1;
                                    if (scalefac_gain[k as usize].wrapping_sub(last_val_scf))
                                        .wrapping_abs()
                                        <= CODE_BOOK_SCF_LAV
                                    {
                                        delta_scf = 0; // save bits
                                    } else {
                                        // do not save bits
                                        delta_scf =
                                            last_val_scf.wrapping_sub(scalefac_gain[j as usize]);
                                        last_val_scf = scalefac_gain[j as usize];
                                        scf_skip_counter = 0;
                                    }
                                    break;
                                }
                                // count scalefactor skip
                                scf_skip_counter += 1;
                            }
                        }

                        // search for the next maxValueInSfb[] != 0 in all other sections
                        let mut m = i + 1;
                        while (m < n_sec) && (found == 0) {
                            let hm = section_data.huffsection[m];
                            if (hm.code_book != CODE_BOOK_ZERO_NO)
                                && (hm.code_book != CODE_BOOK_PNS_NO)
                            {
                                let end = hm.sfb_start + hm.sfb_cnt;
                                for n in hm.sfb_start..end {
                                    if max_value_in_sfb[n as usize] != 0 {
                                        found = 1;
                                        if (scalefac_gain[n as usize].wrapping_sub(last_val_scf))
                                            .wrapping_abs()
                                            <= CODE_BOOK_SCF_LAV
                                        {
                                            delta_scf = 0; // save bits
                                        } else {
                                            // do not save bits
                                            delta_scf = last_val_scf
                                                .wrapping_sub(scalefac_gain[j as usize]);
                                            last_val_scf = scalefac_gain[j as usize];
                                            scf_skip_counter = 0;
                                        }
                                        break;
                                    }
                                    // count scalefactor skip
                                    scf_skip_counter += 1;
                                }
                            }
                            m += 1;
                        }
                        // no maxValueInSfb[] != 0 found
                        if found == 0 {
                            delta_scf = 0;
                            scf_skip_counter = 0;
                        }
                    } else {
                        // consider skipped scalefactors
                        delta_scf = 0;
                        scf_skip_counter -= 1;
                    }
                } else {
                    delta_scf = last_val_scf.wrapping_sub(scalefac_gain[j as usize]);
                    last_val_scf = scalefac_gain[j as usize];
                }
                section_data.scalefac_bits += fdk_aac_enc_bit_count_scalefactor_delta(delta_scf);
            }
        }
    }
}

/// `FDKaacEnc_noiseCount`: count bits used by pns
fn fdk_aac_enc_noise_count(section_data: &mut SectionData, noise_nrg: &[i32]) {
    let mut noise_pcm_flag = TRUE;
    let mut last_val_pns: i32 = 0;

    section_data.noise_nrg_bits = 0;

    for i in 0..section_data.no_of_sections as usize {
        let hs = section_data.huffsection[i];
        if hs.code_book == CODE_BOOK_PNS_NO {
            let sfb_start = hs.sfb_start;
            let sfb_end = sfb_start + hs.sfb_cnt;
            for j in sfb_start..sfb_end {
                if noise_pcm_flag != 0 {
                    section_data.noise_nrg_bits += PNS_PCM_BITS;
                    last_val_pns = noise_nrg[j as usize];
                    noise_pcm_flag = FALSE;
                } else {
                    let delta_pns = noise_nrg[j as usize].wrapping_sub(last_val_pns);
                    last_val_pns = noise_nrg[j as usize];
                    section_data.noise_nrg_bits +=
                        fdk_aac_enc_bit_count_scalefactor_delta(delta_pns);
                }
            }
        }
    }
}

/// `FDKaacEnc_dynBitCount`: sections the quantized spectrum and returns the number of bits of
/// the Huffman coded spectrum, the section side info, the scalefactors and the noise energies.
///
/// `scalefac` is `None` for `NULL`.
#[allow(clippy::too_many_arguments)]
pub fn fdk_aac_enc_dyn_bit_count(
    h_bc: &mut BitcntrState,
    quant_spectrum: &[i16],
    max_value_in_sfb: &[u32],
    scalefac: Option<&[i32]>,
    block_type: i32,
    sfb_cnt: i32,
    max_sfb_per_group: i32,
    sfb_per_group: i32,
    sfb_offset: &[i32],
    section_data: &mut SectionData,
    noise_nrg: &[i32],
    is_book: &[i32],
    is_scale: &[i32],
    syntax_flags: u32,
) -> i32 {
    section_data.block_type = block_type;
    section_data.sfb_cnt = sfb_cnt;
    section_data.sfb_per_group = sfb_per_group;
    // C: sfbCnt / sfbPerGroup (callers never pass sfbPerGroup == 0)
    section_data.no_of_groups =
        if sfb_per_group != 0 { sfb_cnt.wrapping_div(sfb_per_group) } else { 0 };
    section_data.max_sfb_per_group = max_sfb_per_group;

    let BitcntrState { bit_look_up, merge_gain_look_up } = h_bc;
    fdk_aac_enc_noiseless_counter(
        section_data,
        merge_gain_look_up,
        bit_look_up,
        quant_spectrum,
        max_value_in_sfb,
        sfb_offset,
        block_type,
        noise_nrg,
        is_book,
        if (syntax_flags & AC_ER_VCB11) != 0 { 1 } else { 0 },
    );

    fdk_aac_enc_scf_count(scalefac, max_value_in_sfb, section_data, is_scale);

    fdk_aac_enc_noise_count(section_data, noise_nrg);

    section_data.huffman_bits
        + section_data.side_info_bits
        + section_data.scalefac_bits
        + section_data.noise_nrg_bits
}

/// `FDKaacEnc_BCNew(phBC, dynamic_RAM)`: allocates the bit counter state. In C the state is a
/// static pool and the look-up tables point into the shared dynamic RAM; here `ph_bc` (owned by
/// `QcState::h_bit_counter`) gets its own zeroed tables. Returns 0.
pub fn fdk_aac_enc_bc_new(ph_bc: &mut BitcntrState) -> i32 {
    *ph_bc = BitcntrState::default();
    0
}

/// `FDKaacEnc_BCClose(phBC)`: releases the look-up tables.
pub fn fdk_aac_enc_bc_close(ph_bc: &mut BitcntrState) {
    ph_bc.bit_look_up = Vec::new();
    ph_bc.merge_gain_look_up = Vec::new();
}

// The helpers on fixed-size rows against the straight loops (`0..=CODE_BOOK_ESC_NDX` over
// slices), on random rows with the extremes (0 and INVALID_BITCOUNT).
#[cfg(test)]
mod tests {
    use super::*;

    fn find_best_book_straight(bc: &[i32], book: &mut i32) -> i32 {
        let mut min_bits = INVALID_BITCOUNT;
        for j in 0..=CODE_BOOK_ESC_NDX {
            if bc[j as usize] < min_bits {
                min_bits = bc[j as usize];
                *book = j;
            }
        }
        min_bits
    }

    fn find_min_merge_bits_straight(bc1: &[i32], bc2: &[i32]) -> i32 {
        let mut min_bits = INVALID_BITCOUNT;
        for j in 0..=CODE_BOOK_ESC_NDX as usize {
            min_bits = fix_min(min_bits, bc1[j] + bc2[j]);
        }
        min_bits
    }

    fn merge_bit_look_up_straight(bit_look_up: &mut [i32], ndx1: i32, ndx2: i32) {
        let (a, b) = (ndx1 as usize * BIT_LOOK_UP_WIDTH, ndx2 as usize * BIT_LOOK_UP_WIDTH);
        for j in 0..=CODE_BOOK_ESC_NDX as usize {
            bit_look_up[a + j] = fix_min(bit_look_up[a + j] + bit_look_up[b + j], INVALID_BITCOUNT);
        }
    }

    #[test]
    fn row_helpers_vs_straight() {
        let mut seed = 0x2545_F491_4F6C_DD1Du64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed >> 33) as i32
        };
        for _ in 0..100_000 {
            let mut t: Vec<i32> = (0..4 * BIT_LOOK_UP_WIDTH)
                .map(|_| match next() % 4 {
                    0 => INVALID_BITCOUNT,
                    1 => 0,
                    2 => next() % 64,
                    _ => next() % (INVALID_BITCOUNT + 1),
                })
                .collect();
            let (i, k) = (next() % 4, next() % 4);
            // the book keeps its value when no entry is below INVALID_BITCOUNT
            let mut b0 = next();
            let mut b1 = b0;
            let s = find_best_book_straight(row(&t, i), &mut b0);
            let f = fdk_aac_enc_find_best_book(row(&t, i), &mut b1, 0);
            assert_eq!((s, b0), (f, b1));
            assert_eq!(
                find_min_merge_bits_straight(row(&t, i), row(&t, k)),
                fdk_aac_enc_find_min_merge_bits(row(&t, i), row(&t, k), 0)
            );
            let mut u = t.clone();
            merge_bit_look_up_straight(&mut t, i, k);
            fdk_aac_enc_merge_bit_look_up(&mut u, i, k);
            assert_eq!(t, u);
        }
    }
}
