// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACdec/src/aacdec_hcrs.h, libAACdec/src/aacdec_hcrs.cpp.

//! Port of `aacdec_hcrs.h`: HCR decoder: decoding of the non-priority codewords (non-PCWs),
//! segmentation and bit field handling, and the HCR state machine.
//!
//! A `STATEFUNC` is represented by its state constant ([`StateFunc`], see
//! [`crate::aacdec_hcr_types`]); [`hcr_state`] calls the state function of a constant. The state
//! functions take the spectrum of the channel (`nonPcwSideinfo.pResultBase`) as `spec`.
//!
//! The file-local (`static`) functions of `aacdec_hcrs.cpp` are public here so that the
//! differential tests can reach them. Indices outside the arrays are handled as described in
//! [`crate::aacdec_hcr`].

use fdk_aac_rs_fdk::bitstream::FdkBitStream;
use fdk_aac_rs_fdk::common_fix::{fixp_abs, FixpDbl};

use crate::aac_rom::{
    A_CODEBOOK_2_START_INT, A_DIM_CB, A_HUFF_TABLE, A_QUANT_TABLE, A_STATE_CONSTANT_2_STATE,
};
use crate::aacdec_hcr::{carry_bit_to_branch_value, rd, wr};
use crate::aacdec_hcr_bit::{hcr_get_a_bit_from_bitstream, toggle_read_direction};
use crate::aacdec_hcr_types::*;

/* different states are defined as constants */
/// `STOP_THIS_STATE`
pub const STOP_THIS_STATE: StateFunc = 0;
/// `BODY_ONLY`
pub const BODY_ONLY: StateFunc = 1;
/// `BODY_SIGN__BODY`
pub const BODY_SIGN__BODY: StateFunc = 2;
/// `BODY_SIGN__SIGN`
pub const BODY_SIGN__SIGN: StateFunc = 3;
/// `BODY_SIGN_ESC__BODY`
pub const BODY_SIGN_ESC__BODY: StateFunc = 4;
/// `BODY_SIGN_ESC__SIGN`
pub const BODY_SIGN_ESC__SIGN: StateFunc = 5;
/// `BODY_SIGN_ESC__ESC_PREFIX`
pub const BODY_SIGN_ESC__ESC_PREFIX: StateFunc = 6;
/// `BODY_SIGN_ESC__ESC_WORD`
pub const BODY_SIGN_ESC__ESC_WORD: StateFunc = 7;

/// `MSB_31_MASK`: masks MSB (= Bit 31) in a 32 bit word
pub const MSB_31_MASK: u32 = 0x80000000;
/// `DIMENSION_OF_ESCAPE_CODEBOOK`: for cb >= 11 is dimension 2
pub const DIMENSION_OF_ESCAPE_CODEBOOK: i32 = 2;
/// `ESCAPE_CODEBOOK`
pub const ESCAPE_CODEBOOK: i32 = 11;

/// `MASK_ESCAPE_PREFIX_UP`
pub const MASK_ESCAPE_PREFIX_UP: u32 = 0x000F0000;
/// `LSB_ESCAPE_PREFIX_UP`
pub const LSB_ESCAPE_PREFIX_UP: u32 = 16;

/// `MASK_ESCAPE_PREFIX_DOWN`
pub const MASK_ESCAPE_PREFIX_DOWN: u32 = 0x0000F000;
/// `LSB_ESCAPE_PREFIX_DOWN`
pub const LSB_ESCAPE_PREFIX_DOWN: u32 = 12;

/// `MASK_ESCAPE_WORD`
pub const MASK_ESCAPE_WORD: u32 = 0x00000FFF;
/// `MASK_FLAG_A`
pub const MASK_FLAG_A: u32 = 0x00200000;
/// `MASK_FLAG_B`
pub const MASK_FLAG_B: u32 = 0x00100000;

/// `((STATEFUNC)pState)(bs, pHcr)`: calls the state function of the state constant `state`
/// (`aStateConstant2State[state]`). A constant without a function (0, `NULL` in C, which is
/// never called) returns 0.
pub fn hcr_state(
    state: StateFunc,
    bs: &mut FdkBitStream,
    p_hcr: &mut CErHcrInfo,
    spec: &mut [FixpDbl],
) -> u32 {
    match state {
        BODY_ONLY => hcr_state_body_only(bs, p_hcr, spec),
        BODY_SIGN__BODY => hcr_state_body_sign__body(bs, p_hcr, spec),
        BODY_SIGN__SIGN => hcr_state_body_sign__sign(bs, p_hcr, spec),
        BODY_SIGN_ESC__BODY => hcr_state_body_sign_esc__body(bs, p_hcr, spec),
        BODY_SIGN_ESC__SIGN => hcr_state_body_sign_esc__sign(bs, p_hcr, spec),
        BODY_SIGN_ESC__ESC_PREFIX => hcr_state_body_sign_esc__esc_prefix(bs, p_hcr, spec),
        BODY_SIGN_ESC__ESC_WORD => hcr_state_body_sign_esc__esc_word(bs, p_hcr, spec),
        _ => 0,
    }
}

/// `aStateConstant2State[i]` for an `SCHAR` index of `pSta` (0..=7 for every value the HCR code
/// writes; other values select `NULL`).
#[inline]
fn state_of(i: i8) -> StateFunc {
    rd(&A_STATE_CONSTANT_2_STATE, i as usize)
}

/// `DecodeNonPCWs(bs, pHcr)`: decodes all non-priority codewords (non-PCWs) with the state
/// machine.
///
/// `spec` is the spectrum of the channel (`nonPcwSideinfo.pResultBase`).
pub fn decode_non_pcws(bs: &mut FdkBitStream, p_hcr: &mut CErHcrInfo, spec: &mut [FixpDbl]) {
    let num_valid_segment;
    {
        let sg = &mut p_hcr.segment_info;
        num_valid_segment = init_segment_bitfield(
            &sg.num_segment,
            &sg.p_remaining_bits_in_segment,
            &mut sg.p_segment_bitfield,
            &mut sg.num_word_for_bitfield,
            &mut sg.p_num_bit_valid_in_last_word,
        );
    }

    if num_valid_segment != 0 {
        let num_segment = p_hcr.segment_info.num_segment;
        let mut num_codeword = p_hcr.section_info.num_codeword;
        // numSet = ((numCodeword - 1) / *pNumSegment) + 1, truncated to UCHAR
        let mut num_set: u8 = ((num_codeword.wrapping_sub(1) / num_segment).wrapping_add(1)) as u8;

        p_hcr.segment_info.read_direction = FROM_RIGHT_TO_LEFT as u8;

        /* Process sets subsequently */
        num_set = num_set.min(MAX_HCR_SETS as u8);
        let mut current_set: u8 = 1;
        while current_set < num_set {
            /* step 1 */
            /* number of remaining non PCWs [for all sets] */
            num_codeword = num_codeword.wrapping_sub(num_segment);
            let mut codeword_in_set: u32 = if num_codeword < num_segment {
                num_codeword /* for last set */
            } else {
                num_segment /* for all sets except last set */
            };

            /* step 2 */
            /* prepare array 'CodewordBitfield'; as much ones are written from left in all
            words, as much decodedCodewordInSetCounter nonPCWs exist in this set */
            let mut temp_word: u32 = 0xFFFFFFFF;
            let num_word_for_bitfield = p_hcr.segment_info.num_word_for_bitfield;
            {
                let cw_bf = &mut p_hcr.segment_info.p_codeword_bitfield;
                let mut idx = 0usize;
                let mut bitfield_word = num_word_for_bitfield as i32;
                while bitfield_word != 0 {
                    /* loop over all used words */
                    if codeword_in_set > NUMBER_OF_BIT_IN_WORD as u32 {
                        /* more codewords than number of bits => fill ones */
                        /* fill a whole word with ones */
                        wr(cw_bf, idx, temp_word);
                        idx += 1;
                        codeword_in_set -= NUMBER_OF_BIT_IN_WORD as u32; /* subtract number of bits */
                    } else {
                        /* prepare last tempWord */
                        let mut remaining_codewords_in_set = codeword_in_set;
                        while remaining_codewords_in_set < NUMBER_OF_BIT_IN_WORD as u32 {
                            /* set a zero at bit number (NUMBER_OF_BIT_IN_WORD-1-i) in tempWord */
                            temp_word &= !(1u32
                                << (NUMBER_OF_BIT_IN_WORD as u32 - 1 - remaining_codewords_in_set));
                            remaining_codewords_in_set += 1;
                        }
                        wr(cw_bf, idx, temp_word);
                        idx += 1;
                        temp_word = 0x00000000;
                    }
                    bitfield_word -= 1;
                }
            }

            /* step 3 */
            /* build non-PCW sideinfo for each non-PCW of the current set */
            init_non_pcw_side_information_for_current_set(p_hcr);

            /* step 4 */
            /* decode all non-PCWs belonging to this set */

            /* loop over trials */
            let mut codeword_offset_base: i32 = 0;
            let mut trial = num_segment;
            while trial > 0 {
                /* loop over number of words in bitfields */
                let mut segment_offset: i32 = 0; /* start at zero in every segment */
                p_hcr.segment_info.segment_offset = segment_offset as u32; /* store in structure for states */
                let mut codeword_offset: i32 = codeword_offset_base;
                p_hcr.non_pcw_sideinfo.codeword_offset = codeword_offset as u32; /* store in structure for states */

                let mut bitfield_word: i32 = 0;
                while bitfield_word < num_word_for_bitfield as i32 {
                    /* derive tempWord with bitwise and */
                    let tw = rd(&p_hcr.segment_info.p_segment_bitfield, bitfield_word as usize)
                        & rd(&p_hcr.segment_info.p_codeword_bitfield, bitfield_word as usize);

                    /* if tempWord is not zero, decode something */
                    if tw != 0 {
                        /* loop over all bits in tempWord; start state machine if & is true */
                        let mut bit_in_word: i32 = NUMBER_OF_BIT_IN_WORD;
                        while bit_in_word > 0 {
                            let inter_mediate_word: u32 = 1u32 << (bit_in_word - 1);
                            if (tw & inter_mediate_word) == inter_mediate_word {
                                /* get state and start state machine */
                                p_hcr.non_pcw_sideinfo.p_state = state_of(rd(
                                    &p_hcr.non_pcw_sideinfo.p_sta,
                                    codeword_offset as usize,
                                ));

                                while p_hcr.non_pcw_sideinfo.p_state != 0 {
                                    let ret =
                                        hcr_state(p_hcr.non_pcw_sideinfo.p_state, bs, p_hcr, spec);
                                    if ret != 0 {
                                        return;
                                    }
                                }
                            }

                            /* update both offsets */
                            segment_offset += 1; /* add NUMBER_OF_BIT_IN_WORD times one */
                            p_hcr.segment_info.segment_offset = segment_offset as u32;
                            codeword_offset += 1; /* add NUMBER_OF_BIT_IN_WORD times one */
                            /* index of the current codeword lies within modulo range */
                            codeword_offset = modulo_value(codeword_offset, num_segment as i32);
                            p_hcr.non_pcw_sideinfo.codeword_offset = codeword_offset as u32;
                            bit_in_word -= 1;
                        }
                    } else {
                        segment_offset += NUMBER_OF_BIT_IN_WORD; /* add NUMBER_OF_BIT_IN_WORD at once */
                        p_hcr.segment_info.segment_offset = segment_offset as u32;
                        codeword_offset += NUMBER_OF_BIT_IN_WORD; /* add NUMBER_OF_BIT_IN_WORD at once */
                        /* index of the current codeword lies within modulo range */
                        codeword_offset = modulo_value(codeword_offset, num_segment as i32);
                        p_hcr.non_pcw_sideinfo.codeword_offset = codeword_offset as u32;
                    }
                    bitfield_word += 1;
                } /* end of bitfield word loop */

                /* decrement codeword - pointer */
                codeword_offset_base -= 1;
                /* index of the current codeword base lies within modulo range */
                codeword_offset_base = modulo_value(codeword_offset_base, num_segment as i32);

                /* rotate numSegment bits in codewordBitfield */
                /* rotation of *numSegment bits in bitfield of codewords (circle-rotation) */
                let nw = num_word_for_bitfield as usize;
                let nbv = p_hcr.segment_info.p_num_bit_valid_in_last_word as u32;
                let cw_bf = &mut p_hcr.segment_info.p_codeword_bitfield;
                // The shift amounts are 0..=31 (pNumBitValidInLastWord is 1..=32).
                let sh = (NUMBER_OF_BIT_IN_WORD as u32).wrapping_sub(nbv) & 31;
                /* get last valid bit */
                // tempBit = word & (1 << sh); tempBit = tempBit >> sh (an INT: bit 31 gives -1,
                // and (-1) << 31 below gives the same word as 1 << 31)
                let last = rd(cw_bf, nw.wrapping_sub(1));
                let temp_bit: u32 = (last >> sh) & 1;

                /* write zero into place where tempBit was fetched from */
                let last = last & !(1u32 << sh);

                /* rotate last valid word */
                wr(cw_bf, nw.wrapping_sub(1), last >> 1);

                /* transfare carry bit 0 from current word into bitposition 31 from next word and
                rotate current word */
                let mut bitfield_word = nw as i32 - 2;
                while bitfield_word > -1 {
                    let bw = bitfield_word as usize;
                    /* get carry (=bit at position 0) from current word */
                    let carry = rd(cw_bf, bw) & 1;

                    /* put the carry bit at position 31 into word right from current word */
                    let next = rd(cw_bf, bw + 1) | (carry << (NUMBER_OF_BIT_IN_WORD - 1));
                    wr(cw_bf, bw + 1, next);

                    /* shift current word */
                    let cur = rd(cw_bf, bw) >> 1;
                    wr(cw_bf, bw, cur);
                    bitfield_word -= 1;
                }

                /* put tempBit into free bit-position 31 from first word */
                let first = rd(cw_bf, 0) | (temp_bit << (NUMBER_OF_BIT_IN_WORD - 1));
                wr(cw_bf, 0, first);

                trial -= 1;
            } /* end of trial loop */

            /* toggle read direction */
            p_hcr.segment_info.read_direction =
                toggle_read_direction(p_hcr.segment_info.read_direction);
            current_set += 1;
        }
        /* end of set loop */

        /* all non-PCWs of this spectrum are decoded */
    }

    /* all PCWs and all non PCWs are decoded. They are unbacksorted in output buffer. Here is the
    Interface with comparing QSCs to asm decoding */
}

/// `InitSegmentBitfield(pNumSegment, pRemainingBitsInSegment, pSegmentBitfield,
/// pNumWordForBitfield, pNumBitValidInLastWord)`: prepares the bit field of the segments (set up
/// once for all sets; a bit is removed when its segment is decoded empty). Returns the number of
/// valid (non-empty) segments.
pub fn init_segment_bitfield(
    p_num_segment: &u32,
    p_remaining_bits_in_segment: &[i8],
    p_segment_bitfield: &mut [u32],
    p_num_word_for_bitfield: &mut u8,
    p_num_bit_valid_in_last_word: &mut u16,
) -> u32 {
    *p_num_word_for_bitfield = if *p_num_segment == 0 {
        0
    } else {
        // ((*pNumSegment - 1) >> 5) + 1 in UINT, stored into a UCHAR
        ((*p_num_segment - 1) >> THIRTYTWO_LOG_DIV_TWO_LOG).wrapping_add(1) as u8
    };

    /* loop over all words, which are completely used or only partial */
    /* bit in pSegmentBitfield is zero if segment is empty; bit in pSegmentBitfield is one if
    segment is not empty */
    let mut num_valid_segment: u16 = 0;
    *p_num_bit_valid_in_last_word = *p_num_segment as u16;

    /* loop over words */
    let mut bitfield_word: u8 = 0;
    while (bitfield_word as i32) < *p_num_word_for_bitfield as i32 - 1 {
        let mut temp_word: u32 = 0xFFFFFFFF; /* set ones */
        let r = (bitfield_word as u16) << THIRTYTWO_LOG_DIV_TWO_LOG;
        for i in 0..NUMBER_OF_BIT_IN_WORD as usize {
            if rd(p_remaining_bits_in_segment, r as usize + i) == 0 {
                /* set a zero at bit number (NUMBER_OF_BIT_IN_WORD-1-i) in tempWord */
                temp_word &= !(1u32 << (NUMBER_OF_BIT_IN_WORD as usize - 1 - i));
            } else {
                num_valid_segment = num_valid_segment.wrapping_add(1); /* count segments which are not empty */
            }
        }
        wr(p_segment_bitfield, bitfield_word as usize, temp_word); /* store result */
        /* calculate number of zeros on LSB side in the last word */
        *p_num_bit_valid_in_last_word =
            p_num_bit_valid_in_last_word.wrapping_sub(NUMBER_OF_BIT_IN_WORD as u16);
        bitfield_word = bitfield_word.wrapping_add(1);
    }

    /* calculate last word: prepare special tempWord */
    let mut temp_word: u32 = 0xFFFFFFFF;
    let mut i: i32 = 0;
    while i < (NUMBER_OF_BIT_IN_WORD - *p_num_bit_valid_in_last_word as i32) {
        // 1 << i for i = 0..=31 (pNumBitValidInLastWord is at least 0)
        temp_word &= !(1u32.wrapping_shl(i as u32)); /* clear bit i in tempWord */
        i += 1;
    }

    /* calculate last word */
    let r = (bitfield_word as u16) << THIRTYTWO_LOG_DIV_TWO_LOG;
    let mut i: i32 = 0;
    while i < *p_num_bit_valid_in_last_word as i32 {
        if rd(p_remaining_bits_in_segment, r as usize + i as usize) == 0 {
            /* set a zero at bit number (NUMBER_OF_BIT_IN_WORD-1-i) in tempWord */
            temp_word &= !(1u32.wrapping_shl((NUMBER_OF_BIT_IN_WORD - 1 - i) as u32));
        } else {
            num_valid_segment = num_valid_segment.wrapping_add(1); /* count segments which are not empty */
        }
        i += 1;
    }
    wr(p_segment_bitfield, bitfield_word as usize, temp_word); /* store result */

    num_valid_segment as u32
}

/// `InitNonPCWSideInformationForCurrentSet(pHcr)`: sets up the side info for the non-PCW decoder
/// (for the current set).
pub fn init_non_pcw_side_information_for_current_set(p_hcr: &mut CErHcrInfo) {
    let si = &mut p_hcr.section_info;
    let np = &mut p_hcr.non_pcw_sideinfo;
    let mut num_extended_sorted_codeword_in_section_idx =
        si.num_extended_sorted_codeword_in_section_idx;
    let mut extended_sorted_codebook_idx = si.extended_sorted_codebook_idx;
    let mut num_extended_sorted_sections_in_sets_idx = si.num_extended_sorted_sections_in_sets_idx;
    let mut quantized_spectral_coefficients_idx =
        p_hcr.dec_in_out.quantized_spectral_coefficients_idx;
    let mut iteration_counter: i32 = 0;
    // pCodebook, iNode, pCntSign, iResultPointer, pEscapeSequenceInfo, pSta
    let mut p = 0usize;

    /* loop over number of extended sorted sections in the current set so all codewords sideinfo
    variables within this set can be prepared for decoding */
    let mut i: u16 = rd(
        &si.p_num_extended_sorted_sections_in_sets,
        num_extended_sorted_sections_in_sets_idx as usize,
    );
    while i != 0 {
        let cb = rd(&si.p_extended_sorted_codebook, extended_sorted_codebook_idx as usize);
        // The extended sorted codebooks are below 32.
        let cbi = (cb as usize) & (MAX_CB as usize - 1);
        let codebook_dim: u8 = A_DIM_CB[cbi];
        let start_node: u32 = A_HUFF_TABLE[cbi][0];

        let mut k: u16 = rd(
            &si.p_num_extended_sorted_codeword_in_section,
            num_extended_sorted_codeword_in_section_idx as usize,
        );
        while k != 0 {
            iteration_counter += 1;
            if iteration_counter > (1024 >> 2) {
                return;
            }
            wr(&mut np.p_sta, p, A_CODEBOOK_2_START_INT[cbi]);
            wr(&mut np.p_codebook, p, cb);
            wr(&mut np.i_node, p, start_node);
            wr(&mut np.p_cnt_sign, p, 0);
            wr(&mut np.i_result_pointer, p, quantized_spectral_coefficients_idx as u16);
            wr(&mut np.p_escape_sequence_info, p, 0);
            p += 1;
            /* update pointer by codebookDim --> point to next starting value for writing out */
            quantized_spectral_coefficients_idx += codebook_dim as i32;
            if quantized_spectral_coefficients_idx >= 1024 {
                return;
            }
            k -= 1;
        }
        /* inc ptr for next ext sort sec in current set */
        num_extended_sorted_codeword_in_section_idx += 1;
        /* inc ptr for next ext sort sec in current set */
        extended_sorted_codebook_idx += 1;
        if num_extended_sorted_codeword_in_section_idx >= MAX_SFB_HCR + MAX_HCR_SETS
            || extended_sorted_codebook_idx >= MAX_SFB_HCR + MAX_HCR_SETS
        {
            return;
        }
        i -= 1;
    }
    num_extended_sorted_sections_in_sets_idx += 1; /* inc ptr for next set of non-PCWs */
    if num_extended_sorted_codeword_in_section_idx >= MAX_SFB_HCR + MAX_HCR_SETS {
        return;
    }

    /* Write back indexes */
    si.num_extended_sorted_codeword_in_section_idx = num_extended_sorted_codeword_in_section_idx;
    si.extended_sorted_codebook_idx = extended_sorted_codebook_idx;
    si.num_extended_sorted_sections_in_sets_idx = num_extended_sorted_sections_in_sets_idx;
    si.num_extended_sorted_codeword_in_section_idx = num_extended_sorted_codeword_in_section_idx;
    p_hcr.dec_in_out.quantized_spectral_coefficients_idx = quantized_spectral_coefficients_idx;
}

/// `ModuloValue(input, bufferlength)`: returns the input value if it is in the range of
/// `bufferlength`; one `bufferlength` is added if it is smaller, subtracted if it is bigger.
pub fn modulo_value(input: i32, bufferlength: i32) -> i32 {
    if input > (bufferlength.wrapping_sub(1)) {
        return input.wrapping_sub(bufferlength);
    }
    if input < 0 {
        return input.wrapping_add(bufferlength);
    }
    input
}

/// `ClearBitFromBitfield(ptrState, offset, pBitfield)`: clears a bit in a bit field (of a decoded
/// codeword, or of a segment that is decoded empty) and switches off the state machine.
pub fn clear_bit_from_bitfield(ptr_state: &mut StateFunc, offset: u32, p_bitfield: &mut [u32]) {
    /* get both values needed for clearing the bit */
    let num_bitfield_word = offset >> THIRTYTWO_LOG_DIV_TWO_LOG; /* int   = wordNr */
    let num_bitfield_bit = offset - (num_bitfield_word << THIRTYTWO_LOG_DIV_TWO_LOG); /* fract = bitNr  */

    /* clear a bit in bitfield */
    let w = rd(p_bitfield, num_bitfield_word as usize)
        & !(1u32 << (NUMBER_OF_BIT_IN_WORD as u32 - 1 - num_bitfield_bit));
    wr(p_bitfield, num_bitfield_word as usize, w);

    /* switch off state machine because codeword is decoded and/or because segment is empty */
    *ptr_state = STOP_THIS_STATE;
}

/* =========================================================================================
                           the states of the statemachine
========================================================================================= */

/// Offsets of the current segment and codeword, checked against the arrays of the side info:
/// `segmentOffset < 512` (the segments) and `codewordOffset < 256` (the non-PCW side info). Only
/// set bits of the bit fields start a state, and they belong to codewords inside these arrays;
/// outside them the state machine is switched off.
#[inline]
fn offsets(p_hcr: &mut CErHcrInfo) -> Option<(usize, usize)> {
    let so = p_hcr.segment_info.segment_offset as usize;
    let co = p_hcr.non_pcw_sideinfo.codeword_offset as usize;
    if so < p_hcr.segment_info.p_remaining_bits_in_segment.len()
        && co < p_hcr.non_pcw_sideinfo.p_sta.len()
    {
        Some((so, co))
    } else {
        p_hcr.non_pcw_sideinfo.p_state = STOP_THIS_STATE;
        None
    }
}

/// `HcrGetABitFromBitstream(bs, bsAnchor, &pLeftStartOfSegment[segmentOffset],
/// &pRightStartOfSegment[segmentOffset], readDirection)`
#[inline]
fn get_bit(bs: &mut FdkBitStream, p_hcr: &mut CErHcrInfo, so: usize) -> u8 {
    let anchor = p_hcr.dec_in_out.bitstream_anchor;
    let sg = &mut p_hcr.segment_info;
    hcr_get_a_bit_from_bitstream(
        bs,
        anchor,
        &mut sg.p_left_start_of_segment[so],
        &mut sg.p_right_start_of_segment[so],
        sg.read_direction,
    ) as u8
}

/// `pRemainingBitsInSegment[segmentOffset] -= 1` (an `SCHAR`)
#[inline]
fn dec_remaining(p_hcr: &mut CErHcrInfo, so: usize) {
    let r = &mut p_hcr.segment_info.p_remaining_bits_in_segment[so];
    *r = r.wrapping_sub(1);
}

/// The end of every state: if the segment has no bits left, its bit is cleared in the segment
/// bit field and the state machine is switched off; a negative bit count is an error.
#[inline]
fn segment_end(p_hcr: &mut CErHcrInfo, so: usize, error: u32, state: StateFunc) -> u32 {
    if p_hcr.segment_info.p_remaining_bits_in_segment[so] <= 0 {
        /* clear a bit in bitfield and switch off statemachine */
        clear_bit_from_bitfield(
            &mut p_hcr.non_pcw_sideinfo.p_state,
            so as u32,
            &mut p_hcr.segment_info.p_segment_bitfield,
        );

        if p_hcr.segment_info.p_remaining_bits_in_segment[so] < 0 {
            p_hcr.dec_in_out.error_log |= error;
            return state as u32;
        }
    }

    STOP_THIS_STATE as u32
}

/// `Hcr_State_BODY_ONLY(bs, ptr)`: decodes the body of a codeword (codebooks 1, 2, 5 and 6). No
/// sign bits are decoded, because the table of the quantized spectral values has the sign.
pub fn hcr_state_body_only(
    bs: &mut FdkBitStream,
    p_hcr: &mut CErHcrInfo,
    spec: &mut [FixpDbl],
) -> u32 {
    let Some((segment_offset, codeword_offset)) = offsets(p_hcr) else {
        return STOP_THIS_STATE as u32;
    };
    let mut branch_node: u32 = 0;
    let mut branch_value: u32 = 0;

    let cb = (p_hcr.non_pcw_sideinfo.p_codebook[codeword_offset] as usize) & (MAX_CB as usize - 1);
    let mut tree_node = p_hcr.non_pcw_sideinfo.i_node[codeword_offset];
    let p_current_tree: &[u32] = A_HUFF_TABLE[cb];

    while p_hcr.segment_info.p_remaining_bits_in_segment[segment_offset] > 0 {
        let carry_bit = get_bit(bs, p_hcr, segment_offset);

        /* make a step in decoding tree */
        carry_bit_to_branch_value(carry_bit, tree_node, &mut branch_value, &mut branch_node);

        /* if end of branch reached write out lines and count bits needed for sign, otherwise
        store node in codeword sideinfo */
        if (branch_node & TEST_BIT_10 as u32) == TEST_BIT_10 as u32 {
            /* test bit 10 ; ==> body is complete */
            /* get base address of quantized values belonging to current codebook */
            let p_quant_val_base: &[i8] = A_QUANT_TABLE[cb];
            /* set pointer to first valid line [of 2 or 4 quantized values] */
            let p_quant_val = branch_value as usize;

            /* get position of first line for writing out result */
            let mut i_qsc = p_hcr.non_pcw_sideinfo.i_result_pointer[codeword_offset] as usize;

            for k in 0..A_DIM_CB[cb] as usize {
                /* write out 2 or 4 lines into spectrum; no Sign bits available in this state */
                wr(spec, i_qsc, p_quant_val_base[p_quant_val + k] as FixpDbl);
                i_qsc += 1;
            }

            /* clear a bit in bitfield and switch off statemachine */
            clear_bit_from_bitfield(
                &mut p_hcr.non_pcw_sideinfo.p_state,
                segment_offset as u32,
                &mut p_hcr.segment_info.p_codeword_bitfield,
            );
            /* last reinitialzation of for loop counter (see above) is done here */
            dec_remaining(p_hcr, segment_offset);
            break; /* end of branch in tree reached  i.e. a whole nonPCW-Body is decoded */
        } else {
            /* body is not decoded completely: */
            /* update treeNode for further step in decoding tree */
            tree_node = p_current_tree[branch_value as usize];
        }
        dec_remaining(p_hcr, segment_offset);
    }
    /* store updated treeNode because maybe decoding of codeword body not finished yet */
    p_hcr.non_pcw_sideinfo.i_node[codeword_offset] = tree_node;

    segment_end(p_hcr, segment_offset, STATE_ERROR_BODY_ONLY, BODY_ONLY)
}

/// `Hcr_State_BODY_SIGN__BODY(bs, ptr)`: decodes the codeword body, writes out the result and
/// counts the number of quantized spectral values that are different from zero (they need sign
/// bits). If the count is not zero, the next state decodes the sign bits; otherwise the codeword
/// is decoded.
pub fn hcr_state_body_sign__body(
    bs: &mut FdkBitStream,
    p_hcr: &mut CErHcrInfo,
    spec: &mut [FixpDbl],
) -> u32 {
    let Some((segment_offset, codeword_offset)) = offsets(p_hcr) else {
        return STOP_THIS_STATE as u32;
    };
    let mut branch_node: u32 = 0;
    let mut branch_value: u32 = 0;

    let cb = (p_hcr.non_pcw_sideinfo.p_codebook[codeword_offset] as usize) & (MAX_CB as usize - 1);
    let mut tree_node = p_hcr.non_pcw_sideinfo.i_node[codeword_offset];
    let p_current_tree: &[u32] = A_HUFF_TABLE[cb];

    while p_hcr.segment_info.p_remaining_bits_in_segment[segment_offset] > 0 {
        let carry_bit = get_bit(bs, p_hcr, segment_offset);

        /* make a step in decoding tree */
        carry_bit_to_branch_value(carry_bit, tree_node, &mut branch_value, &mut branch_node);

        /* if end of branch reached write out lines and count bits needed for sign, otherwise
        store node in codeword sideinfo */
        if (branch_node & TEST_BIT_10 as u32) == TEST_BIT_10 as u32 {
            /* test bit 10 ; if set body complete */
            /* body completely decoded; branchValue is valid, set pQuantVal to first (of two or
            four) quantized spectral coefficients */
            let p_quant_val_base: &[i8] = A_QUANT_TABLE[cb];
            let mut p_quant_val = branch_value as usize;

            /* get position of first line for writing result */
            let mut i_qsc = p_hcr.non_pcw_sideinfo.i_result_pointer[codeword_offset] as usize;

            /* codeword decoding result is written out here: Write out 2 or 4 quantized spectral
            values with probably wrong sign and count number of values which are different from
            zero for sign bit decoding [which happens in next state] */
            let mut cnt_sign: u32 = 0;
            for _ in 0..A_DIM_CB[cb] {
                let q = p_quant_val_base[p_quant_val];
                /* write quant. spec. coef. into spectrum */
                wr(spec, i_qsc, q as FixpDbl);
                i_qsc += 1;
                p_quant_val += 1;
                if q != 0 {
                    cnt_sign += 1;
                }
            }

            if cnt_sign == 0 {
                /* clear a bit in bitfield and switch off statemachine */
                clear_bit_from_bitfield(
                    &mut p_hcr.non_pcw_sideinfo.p_state,
                    segment_offset as u32,
                    &mut p_hcr.segment_info.p_codeword_bitfield,
                );
            } else {
                /* write sign count result into codewordsideinfo of current codeword */
                p_hcr.non_pcw_sideinfo.p_cnt_sign[codeword_offset] = cnt_sign as u8;
                p_hcr.non_pcw_sideinfo.p_sta[codeword_offset] = BODY_SIGN__SIGN as i8; /* change state */
                /* get state from separate array of cw-sideinfo */
                p_hcr.non_pcw_sideinfo.p_state =
                    state_of(p_hcr.non_pcw_sideinfo.p_sta[codeword_offset]);
            }
            /* last reinitialzation of for loop counter (see above) is done here */
            dec_remaining(p_hcr, segment_offset);
            break; /* end of branch in tree reached  i.e. a whole nonPCW-Body is decoded */
        } else {
            /* body is not decoded completely: */
            /* update treeNode for further step in decoding tree */
            tree_node = p_current_tree[branch_value as usize];
        }
        dec_remaining(p_hcr, segment_offset);
    }
    /* store updated treeNode because maybe decoding of codeword body not finished yet */
    p_hcr.non_pcw_sideinfo.i_node[codeword_offset] = tree_node;

    segment_end(p_hcr, segment_offset, STATE_ERROR_BODY_SIGN__BODY, BODY_SIGN__BODY)
}

/// `Hcr_State_BODY_SIGN__SIGN(bs, ptr)`: decodes the sign bits of a codeword. The state is called
/// in different trials until `pCntSign[codewordOffset]` is zero.
pub fn hcr_state_body_sign__sign(
    bs: &mut FdkBitStream,
    p_hcr: &mut CErHcrInfo,
    spec: &mut [FixpDbl],
) -> u32 {
    let Some((segment_offset, codeword_offset)) = offsets(p_hcr) else {
        return STOP_THIS_STATE as u32;
    };

    let mut i_qsc = p_hcr.non_pcw_sideinfo.i_result_pointer[codeword_offset] as u32;
    let mut cnt_sign: u8 = p_hcr.non_pcw_sideinfo.p_cnt_sign[codeword_offset];

    /* loop for sign bit decoding */
    while p_hcr.segment_info.p_remaining_bits_in_segment[segment_offset] > 0 {
        let carry_bit = get_bit(bs, p_hcr, segment_offset);
        /* decrement sign counter because one sign bit has been read */
        cnt_sign = cnt_sign.wrapping_sub(1);

        /* search for a line (which was decoded in previous state) which is not zero. [This
        value will get a sign] */
        while rd(spec, i_qsc as usize) == 0 {
            i_qsc += 1;
            if i_qsc >= 1024 {
                /* points to current value different from zero */
                return BODY_SIGN__SIGN as u32;
            }
        }

        /* put sign together with line; if carryBit is zero, the sign is ok already; no write
        operation necessary in this case */
        if carry_bit != 0 {
            /* carryBit = 1 --> minus */
            let v = rd(spec, i_qsc as usize).wrapping_neg();
            wr(spec, i_qsc as usize, v);
        }

        i_qsc += 1; /* update pointer to next (maybe valid) value */

        if cnt_sign == 0 {
            /* if (cntSign==0)  ==>  set state CODEWORD_DECODED */
            /* clear a bit in bitfield and switch off statemachine */
            clear_bit_from_bitfield(
                &mut p_hcr.non_pcw_sideinfo.p_state,
                segment_offset as u32,
                &mut p_hcr.segment_info.p_codeword_bitfield,
            );
            /* last reinitialzation of for loop counter (see above) is done here */
            dec_remaining(p_hcr, segment_offset);
            break; /* whole nonPCW-Body and according sign bits are decoded */
        }
        dec_remaining(p_hcr, segment_offset);
    }
    p_hcr.non_pcw_sideinfo.p_cnt_sign[codeword_offset] = cnt_sign;
    /* store updated pResultPointer */
    p_hcr.non_pcw_sideinfo.i_result_pointer[codeword_offset] = i_qsc as u16;

    segment_end(p_hcr, segment_offset, STATE_ERROR_BODY_SIGN__SIGN, BODY_SIGN__SIGN)
}

/// `Hcr_State_BODY_SIGN_ESC__BODY(bs, ptr)`: decodes the codeword body for codebook 11 (and the
/// virtual codebooks 16 to 31). Writes out the two lines (sign and value may be wrong; an
/// escape value 16 is corrected by the following states) and counts the lines that need sign
/// bits.
pub fn hcr_state_body_sign_esc__body(
    bs: &mut FdkBitStream,
    p_hcr: &mut CErHcrInfo,
    spec: &mut [FixpDbl],
) -> u32 {
    let Some((segment_offset, codeword_offset)) = offsets(p_hcr) else {
        return STOP_THIS_STATE as u32;
    };
    let mut branch_node: u32 = 0;
    let mut branch_value: u32 = 0;

    let mut tree_node = p_hcr.non_pcw_sideinfo.i_node[codeword_offset];
    let p_current_tree: &[u32] = A_HUFF_TABLE[ESCAPE_CODEBOOK as usize];

    while p_hcr.segment_info.p_remaining_bits_in_segment[segment_offset] > 0 {
        let carry_bit = get_bit(bs, p_hcr, segment_offset);

        /* make a step in tree */
        carry_bit_to_branch_value(carry_bit, tree_node, &mut branch_value, &mut branch_node);

        /* if end of branch reached write out lines and count bits needed for sign, otherwise
        store node in codeword sideinfo */
        if (branch_node & TEST_BIT_10 as u32) == TEST_BIT_10 as u32 {
            /* test bit 10 ; if set body complete */

            /* body completely decoded; branchValue is valid */
            /* set pQuantVol to first (of two or four) quantized spectral coefficients */
            let p_quant_val_base: &[i8] = A_QUANT_TABLE[ESCAPE_CODEBOOK as usize];
            let mut p_quant_val = branch_value as usize;

            /* make backup from original resultPointer in node storage for state
            BODY_SIGN_ESC__SIGN */
            p_hcr.non_pcw_sideinfo.i_node[codeword_offset] =
                p_hcr.non_pcw_sideinfo.i_result_pointer[codeword_offset] as u32;

            /* get position of first line for writing result */
            let mut i_qsc = p_hcr.non_pcw_sideinfo.i_result_pointer[codeword_offset] as usize;

            /* codeword decoding result is written out here: Write out 2 or 4 quantized spectral
            values with probably wrong sign and count number of values which are different from
            zero for sign bit decoding [which happens in next state] */
            let mut cnt_sign: u32 = 0;

            for _ in 0..DIMENSION_OF_ESCAPE_CODEBOOK {
                let q = p_quant_val_base[p_quant_val];
                /* write quant. spec. coef. into spectrum */
                wr(spec, i_qsc, q as FixpDbl);
                i_qsc += 1;
                p_quant_val += 1;
                if q != 0 {
                    cnt_sign += 1;
                }
            }

            if cnt_sign == 0 {
                /* clear a bit in bitfield and switch off statemachine */
                clear_bit_from_bitfield(
                    &mut p_hcr.non_pcw_sideinfo.p_state,
                    segment_offset as u32,
                    &mut p_hcr.segment_info.p_codeword_bitfield,
                );
                /* codeword decoded */
            } else {
                /* write sign count result into codewordsideinfo of current codeword */
                p_hcr.non_pcw_sideinfo.p_cnt_sign[codeword_offset] = cnt_sign as u8;
                p_hcr.non_pcw_sideinfo.p_sta[codeword_offset] = BODY_SIGN_ESC__SIGN as i8; /* change state */
                /* get state from separate array of cw-sideinfo */
                p_hcr.non_pcw_sideinfo.p_state =
                    state_of(p_hcr.non_pcw_sideinfo.p_sta[codeword_offset]);
            }
            /* the last reinitialzation of for loop counter (see above) is done here */
            dec_remaining(p_hcr, segment_offset);
            break; /* end of branch in tree reached  i.e. a whole nonPCW-Body is decoded */
        } else {
            /* body is not decoded completely: */
            /* update treeNode for further step in decoding tree and store updated treeNode
            because maybe no more bits left in segment */
            tree_node = p_current_tree[branch_value as usize];
            p_hcr.non_pcw_sideinfo.i_node[codeword_offset] = tree_node;
        }
        dec_remaining(p_hcr, segment_offset);
    }

    segment_end(p_hcr, segment_offset, STATE_ERROR_BODY_SIGN_ESC__BODY, BODY_SIGN_ESC__BODY)
}

/// `Hcr_State_BODY_SIGN_ESC__SIGN(bs, ptr)`: decodes the sign bits of a codeword of codebook 11.
/// If one or both lines are 16, the flags A and B in `pEscapeSequenceInfo` select the escape
/// sequences that follow.
pub fn hcr_state_body_sign_esc__sign(
    bs: &mut FdkBitStream,
    p_hcr: &mut CErHcrInfo,
    spec: &mut [FixpDbl],
) -> u32 {
    let Some((segment_offset, codeword_offset)) = offsets(p_hcr) else {
        return STOP_THIS_STATE as u32;
    };

    let mut i_qsc = p_hcr.non_pcw_sideinfo.i_result_pointer[codeword_offset] as u32;
    let mut cnt_sign: u8 = p_hcr.non_pcw_sideinfo.p_cnt_sign[codeword_offset];

    /* loop for sign bit decoding */
    while p_hcr.segment_info.p_remaining_bits_in_segment[segment_offset] > 0 {
        let carry_bit = get_bit(bs, p_hcr, segment_offset);

        /* decrement sign counter because one sign bit has been read */
        cnt_sign = cnt_sign.wrapping_sub(1);
        p_hcr.non_pcw_sideinfo.p_cnt_sign[codeword_offset] = cnt_sign;

        /* get a quantized spectral value (which was decoded in previous state) which is not
        zero. [This value will get a sign] */
        while rd(spec, i_qsc as usize) == 0 {
            i_qsc += 1;
            if i_qsc >= 1024 {
                return BODY_SIGN_ESC__SIGN as u32;
            }
        }
        p_hcr.non_pcw_sideinfo.i_result_pointer[codeword_offset] = i_qsc as u16;

        /* put negative sign together with quantized spectral value; if carryBit is zero, the
        sign is ok already; no write operation necessary in this case */
        if carry_bit != 0 {
            /* carryBit = 1 --> minus */
            let v = rd(spec, i_qsc as usize).wrapping_neg();
            wr(spec, i_qsc as usize, v);
        }
        i_qsc += 1; /* update index to next (maybe valid) value */
        p_hcr.non_pcw_sideinfo.i_result_pointer[codeword_offset] = i_qsc as u16;

        if cnt_sign == 0 {
            /* all sign bits are decoded now */
            /* last reinitialzation of for loop counter (see above) is done here */
            dec_remaining(p_hcr, segment_offset);

            /* check decoded values if codeword is decoded: Check if one or two escape sequences
            16 follow */

            /* step 0 */
            /* restore pointer to first decoded quantized value [ = original pResultPointr] from
            index iNode prepared in State_BODY_SIGN_ESC__BODY */
            let mut i_qsc = p_hcr.non_pcw_sideinfo.i_node[codeword_offset];

            /* step 1 */
            /* test first value if escape sequence follows */
            let mut flag_a: u32 = 0; /* for first possible escape sequence */
            if fixp_abs(rd(spec, i_qsc as usize)) == ESCAPE_VALUE as FixpDbl {
                flag_a = 1;
            }
            i_qsc = i_qsc.wrapping_add(1);

            /* step 2 */
            /* test second value if escape sequence follows */
            let mut flag_b: u32 = 0; /* for second possible escape sequence */
            if fixp_abs(rd(spec, i_qsc as usize)) == ESCAPE_VALUE as FixpDbl {
                flag_b = 1;
            }

            /* step 3 */
            /* evaluate flag result and go on if necessary */
            if flag_a == 0 && flag_b == 0 {
                /* clear a bit in bitfield and switch off statemachine */
                clear_bit_from_bitfield(
                    &mut p_hcr.non_pcw_sideinfo.p_state,
                    segment_offset as u32,
                    &mut p_hcr.segment_info.p_codeword_bitfield,
                );
            } else {
                /* at least one of two lines is 16 */
                /* store both flags at correct positions in non PCW codeword sideinfo
                pEscapeSequenceInfo[codewordOffset] */
                let mut flags: u32 = flag_a << POSITION_OF_FLAG_A;
                flags |= flag_b << POSITION_OF_FLAG_B;
                p_hcr.non_pcw_sideinfo.p_escape_sequence_info[codeword_offset] = flags;

                /* set next state */
                p_hcr.non_pcw_sideinfo.p_sta[codeword_offset] = BODY_SIGN_ESC__ESC_PREFIX as i8;
                /* get state from separate array of cw-sideinfo */
                p_hcr.non_pcw_sideinfo.p_state =
                    state_of(p_hcr.non_pcw_sideinfo.p_sta[codeword_offset]);

                /* set result pointer to the first line of the two decoded lines */
                p_hcr.non_pcw_sideinfo.i_result_pointer[codeword_offset] =
                    p_hcr.non_pcw_sideinfo.i_node[codeword_offset] as u16;

                if flag_a == 0 && flag_b != 0 {
                    /* update pResultPointr ==> state Stat_BODY_SIGN_ESC__ESC_WORD writes to
                    correct position. Second value is the one and only escape value */
                    let i_qsc = p_hcr.non_pcw_sideinfo.i_result_pointer[codeword_offset];
                    p_hcr.non_pcw_sideinfo.i_result_pointer[codeword_offset] =
                        i_qsc.wrapping_add(1);
                }
            } /* at least one of two lines is 16 */
            break; /* nonPCW-Body at cb 11 and according sign bits are decoded */
        } /* if ( cntSign == 0 ) */
        dec_remaining(p_hcr, segment_offset);
    } /* loop over remaining Bits in segment */

    segment_end(p_hcr, segment_offset, STATE_ERROR_BODY_SIGN_ESC__SIGN, BODY_SIGN_ESC__SIGN)
}

/// `Hcr_State_BODY_SIGN_ESC__ESC_PREFIX(bs, ptr)`: decodes the escape prefix (ones) of the first
/// or second escape sequence, including the separating zero, which switches to the escape word
/// state.
pub fn hcr_state_body_sign_esc__esc_prefix(
    bs: &mut FdkBitStream,
    p_hcr: &mut CErHcrInfo,
    _spec: &mut [FixpDbl],
) -> u32 {
    let Some((segment_offset, codeword_offset)) = offsets(p_hcr) else {
        return STOP_THIS_STATE as u32;
    };

    let mut escape_prefix_up: u32 =
        (p_hcr.non_pcw_sideinfo.p_escape_sequence_info[codeword_offset] & MASK_ESCAPE_PREFIX_UP)
            >> LSB_ESCAPE_PREFIX_UP;

    /* decode escape prefix */
    while p_hcr.segment_info.p_remaining_bits_in_segment[segment_offset] > 0 {
        let carry_bit = get_bit(bs, p_hcr, segment_offset);
        let info = &mut p_hcr.non_pcw_sideinfo.p_escape_sequence_info[codeword_offset];

        /* count ones and store sum in escapePrefixUp */
        if carry_bit == 1 {
            escape_prefix_up += 1; /* update conter for ones */
            if escape_prefix_up > 8 {
                p_hcr.dec_in_out.error_log |= STATE_ERROR_BODY_SIGN_ESC__ESC_PREFIX;
                return BODY_SIGN_ESC__ESC_PREFIX as u32;
            }

            /* store updated counter in sideinfo of current codeword */
            *info &= !MASK_ESCAPE_PREFIX_UP; /* delete old escapePrefixUp */
            escape_prefix_up <<= LSB_ESCAPE_PREFIX_UP; /* shift to correct position */
            *info |= escape_prefix_up; /* insert new escapePrefixUp */
            escape_prefix_up >>= LSB_ESCAPE_PREFIX_UP; /* shift back down */
        } else {
            /* separator [zero] reached */
            /* last reinitialzation of for loop counter (see above) is done here */
            dec_remaining(p_hcr, segment_offset);
            let info = &mut p_hcr.non_pcw_sideinfo.p_escape_sequence_info[codeword_offset];
            escape_prefix_up += 4; /* if escape_separator '0' appears, add 4 and ==> break */

            /* store escapePrefixUp in pEscapeSequenceInfo[codewordOffset] at bit position
            escapePrefixUp */
            *info &= !MASK_ESCAPE_PREFIX_UP; /* delete old escapePrefixUp */
            escape_prefix_up <<= LSB_ESCAPE_PREFIX_UP; /* shift to correct position */
            *info |= escape_prefix_up; /* insert new escapePrefixUp */
            escape_prefix_up >>= LSB_ESCAPE_PREFIX_UP; /* shift back down */

            /* store escapePrefixUp in pEscapeSequenceInfo[codewordOffset] at bit position
            escapePrefixDown */
            *info &= !MASK_ESCAPE_PREFIX_DOWN; /* delete old escapePrefixDown */
            escape_prefix_up <<= LSB_ESCAPE_PREFIX_DOWN; /* shift to correct position */
            *info |= escape_prefix_up; /* insert new escapePrefixDown */

            p_hcr.non_pcw_sideinfo.p_sta[codeword_offset] = BODY_SIGN_ESC__ESC_WORD as i8; /* set next state */
            /* get state from separate array of cw-sideinfo */
            p_hcr.non_pcw_sideinfo.p_state =
                state_of(p_hcr.non_pcw_sideinfo.p_sta[codeword_offset]);
            break;
        }
        dec_remaining(p_hcr, segment_offset);
    }

    segment_end(
        p_hcr,
        segment_offset,
        STATE_ERROR_BODY_SIGN_ESC__ESC_PREFIX,
        BODY_SIGN_ESC__ESC_PREFIX,
    )
}

/// `Hcr_State_BODY_SIGN_ESC__ESC_WORD(bs, ptr)`: decodes the escape word of an escape sequence.
/// When the escape sequence is complete, the escape value replaces the decoded 16; if flag B is
/// set, the second escape sequence follows, otherwise the codeword is decoded.
pub fn hcr_state_body_sign_esc__esc_word(
    bs: &mut FdkBitStream,
    p_hcr: &mut CErHcrInfo,
    spec: &mut [FixpDbl],
) -> u32 {
    let Some((segment_offset, codeword_offset)) = offsets(p_hcr) else {
        return STOP_THIS_STATE as u32;
    };

    let info0 = p_hcr.non_pcw_sideinfo.p_escape_sequence_info[codeword_offset];
    let mut escape_word: u32 = info0 & MASK_ESCAPE_WORD;
    let mut escape_prefix_down: u32 = (info0 & MASK_ESCAPE_PREFIX_DOWN) >> LSB_ESCAPE_PREFIX_DOWN;

    /* decode escape word */
    while p_hcr.segment_info.p_remaining_bits_in_segment[segment_offset] > 0 {
        let carry_bit = get_bit(bs, p_hcr, segment_offset);
        let info = &mut p_hcr.non_pcw_sideinfo.p_escape_sequence_info[codeword_offset];

        /* build escape word */
        escape_word <<= 1; /* left shift previous decoded part of escapeWord by on bit */
        escape_word |= carry_bit as u32; /* assemble escape word by bitwise or */

        /* decrement counter for length of escape word because one more bit was decoded */
        escape_prefix_down = escape_prefix_down.wrapping_sub(1);

        /* store updated escapePrefixDown */
        *info &= !MASK_ESCAPE_PREFIX_DOWN; /* delete old escapePrefixDown */
        escape_prefix_down <<= LSB_ESCAPE_PREFIX_DOWN; /* shift to correct position */
        *info |= escape_prefix_down; /* insert new escapePrefixDown */
        escape_prefix_down >>= LSB_ESCAPE_PREFIX_DOWN; /* shift back */

        /* store updated escapeWord */
        *info &= !MASK_ESCAPE_WORD; /* delete old escapeWord */
        *info |= escape_word; /* insert new escapeWord */

        if escape_prefix_down == 0 {
            /* last reinitialzation of for loop counter (see above) is done here */
            dec_remaining(p_hcr, segment_offset);

            /* escape sequence decoded. Assemble escape-line and replace original line */

            /* step 0 */
            /* derive sign */
            let mut i_qsc = p_hcr.non_pcw_sideinfo.i_result_pointer[codeword_offset] as u32;
            /* get sign of escape value 16 */
            let sign: i32 = if rd(spec, i_qsc as usize) >= 0 { 1 } else { -1 };

            /* step 1 */
            /* get escapePrefixUp */
            let info = p_hcr.non_pcw_sideinfo.p_escape_sequence_info[codeword_offset];
            let escape_prefix_up: u32 = (info & MASK_ESCAPE_PREFIX_UP) >> LSB_ESCAPE_PREFIX_UP;

            /* step 2 */
            /* calculate escape value */
            // sign * (((INT)1 << escapePrefixUp) + (INT)escapeWord) (escapePrefixUp is at most
            // 15; the sum can only overflow with an escape word that has wrapped)
            wr(
                spec,
                i_qsc as usize,
                sign.wrapping_mul((1i32 << escape_prefix_up).wrapping_add(escape_word as i32)),
            );

            /* get both flags from sideinfo (flags are not shifted to the lsb-position) */
            let flag_a = info & MASK_FLAG_A;
            let flag_b = info & MASK_FLAG_B;

            /* step 3 */
            /* clear the whole escape sideinfo word */
            p_hcr.non_pcw_sideinfo.p_escape_sequence_info[codeword_offset] = 0;

            /* change state in dependence of flag flagB */
            if flag_a != 0 {
                /* first escape sequence decoded; previous decoded 16 has been replaced by valid
                line */

                /* clear flagA in sideinfo word because this escape sequence has already beed
                decoded */
                p_hcr.non_pcw_sideinfo.p_escape_sequence_info[codeword_offset] &= !MASK_FLAG_A;

                if flag_b == 0 {
                    /* clear a bit in bitfield and switch off statemachine */
                    clear_bit_from_bitfield(
                        &mut p_hcr.non_pcw_sideinfo.p_state,
                        segment_offset as u32,
                        &mut p_hcr.segment_info.p_codeword_bitfield,
                    );
                } else {
                    /* updated pointer to next and last 16 */
                    i_qsc = i_qsc.wrapping_add(1);
                    p_hcr.non_pcw_sideinfo.i_result_pointer[codeword_offset] = i_qsc as u16;

                    /* change state */
                    p_hcr.non_pcw_sideinfo.p_sta[codeword_offset] = BODY_SIGN_ESC__ESC_PREFIX as i8;
                    /* get state from separate array of cw-sideinfo */
                    p_hcr.non_pcw_sideinfo.p_state =
                        state_of(p_hcr.non_pcw_sideinfo.p_sta[codeword_offset]);
                }
            } else {
                /* clear a bit in bitfield and switch off statemachine */
                clear_bit_from_bitfield(
                    &mut p_hcr.non_pcw_sideinfo.p_state,
                    segment_offset as u32,
                    &mut p_hcr.segment_info.p_codeword_bitfield,
                );
            }
            break;
        }
        dec_remaining(p_hcr, segment_offset);
    }

    segment_end(p_hcr, segment_offset, STATE_ERROR_BODY_SIGN_ESC__ESC_WORD, BODY_SIGN_ESC__ESC_WORD)
}
