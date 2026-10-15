// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACdec/src/rvlc.h, libAACdec/src/rvlc.cpp.

//! Port of `rvlc.h`: the RVLC decoder (reversible variable length coding of the scale factors).
//!
//! # Memory
//!
//! The RVLC state (`CErRvlcInfo`) and the scale factor arrays of the forward, backward and escape
//! decoding live in `pComData->overlay.aac` (`erRvlcInfo`, `aRvlcScfFwd`, `aRvlcScfBwd`,
//! `aRvlcScfEsc`). Following the rules of [`crate::channelinfo`], the functions of `rvlc.h` take
//! the channel's [`CAacDecoderCommonData`] (`pAacDecoderChannelInfo->pComData`) as an additional
//! last parameter `com`. The file-local
//! (`static`) functions of `rvlc.cpp` take the parts of it they use; they are public here so that
//! the differential tests can reach them.
//!
//! With the band counts that `IcsRead()` produces (at most 15 bands in each of at most 8 window
//! groups, or one window group of at most 63 bands), every scale factor index `16 * group + band`
//! is below `RVLC_MAX_SFB` (128) and every escape index stays inside `aRvlcScfEsc` (at most 121
//! escapes are used). Indices outside the arrays read 0 and skip the write.

use fdk_aac_rs_fdk::audio::AC_ER_RVLC;
use fdk_aac_rs_fdk::bitstream::{
    fdk_get_valid_bits, fdk_push_bi_directional, fdk_push_for, fdk_read_bits, fdk_sync_cache,
    FdkBitStream,
};
use fdk_aac_rs_fdk::common_fix::{f_max, f_min};
use fdk_aac_rs_fdk::fdk_assert;
use fdk_aac_rs_fdk::mdct::BLOCK_SHORT;

use crate::aac_rom::{A_HUFF_TREE_RVLC_ESCAPE, A_HUFF_TREE_RVL_CODEWDS};
use crate::aacdec_hcr::{carry_bit_to_branch_value, rd, wr};
use crate::aacdec_hcr_types::{CLR_BIT_10, TEST_BIT_10};
use crate::channelinfo::{
    get_scale_factor_bands_transmitted, get_window_groups, get_window_sequence,
    CAacDecoderChannelInfo, CAacDecoderCommonData, CAacDecoderStaticChannelInfo, INTENSITY_HCB,
    INTENSITY_HCB2, NOISE_HCB, ZERO_HCB,
};
use crate::rvlc_info::*;
use crate::rvlcbit::rvlc_read_bit_from_bitstream;
use crate::rvlcconceal::{
    bidirectional_estimation_use_lower_scf_of_current_frame,
    bidirectional_estimation_use_scf_of_prev_frame_as_reference, predictive_interpolation,
    statistical_estimation,
};

/* errorLogRvlc: A word of 32 bits used for logging possible errors within RVLC in case of
distorted bitstreams. */
/// `RVLC_ERROR_ALL_ESCAPE_WORDS_INVALID`: ESC-Dec During RVLC-Escape-decoding there have been
/// more bits decoded as there are available
pub const RVLC_ERROR_ALL_ESCAPE_WORDS_INVALID: u32 = 0x80000000;
/// `RVLC_ERROR_RVL_SUM_BIT_COUNTER_BELOW_ZERO_FWD`: RVL-Dec negative sum-bitcounter during
/// RVL-fwd-decoding (long+shrt)
pub const RVLC_ERROR_RVL_SUM_BIT_COUNTER_BELOW_ZERO_FWD: u32 = 0x40000000;
/// `RVLC_ERROR_RVL_SUM_BIT_COUNTER_BELOW_ZERO_BWD`: RVL-Dec negative sum-bitcounter during
/// RVL-fwd-decoding (long+shrt)
pub const RVLC_ERROR_RVL_SUM_BIT_COUNTER_BELOW_ZERO_BWD: u32 = 0x20000000;
/// `RVLC_ERROR_FORBIDDEN_CW_DETECTED_FWD`: RVL-Dec forbidden codeword detected fwd (long+shrt)
pub const RVLC_ERROR_FORBIDDEN_CW_DETECTED_FWD: u32 = 0x08000000;
/// `RVLC_ERROR_FORBIDDEN_CW_DETECTED_BWD`: RVL-Dec forbidden codeword detected bwd (long+shrt)
pub const RVLC_ERROR_FORBIDDEN_CW_DETECTED_BWD: u32 = 0x04000000;

/// `rvlcInit(pRvlc, pAacDecoderChannelInfo, bs)`: initializes RVLC from the channel info decoded
/// before and sets up the bitstream indices (common initialization part 2 of 2).
///
/// `p_scf_esc`, `p_scf_fwd` and `p_scf_bwd` are `pComData->overlay.aac.aRvlcScfEsc`,
/// `aRvlcScfFwd` and `aRvlcScfBwd`.
pub fn rvlc_init(
    p_rvlc: &mut CErRvlcInfo,
    p_aac_decoder_channel_info: &mut CAacDecoderChannelInfo,
    bs: &mut FdkBitStream,
    p_scf_esc: &mut [i16],
    p_scf_fwd: &mut [i16],
    p_scf_bwd: &mut [i16],
) {
    /* RVLC common initialization part 2 of 2 */
    let p_dyn_data = &mut *p_aac_decoder_channel_info.p_dyn_data;

    p_dyn_data.specific_to.aac.rvlc_intensity_used = 0;

    p_rvlc.num_decoded_escape_words_esc = 0;
    p_rvlc.num_decoded_escape_words_fwd = 0;
    p_rvlc.num_decoded_escape_words_bwd = 0;

    p_rvlc.intensity_used = 0;
    p_rvlc.error_log_rvlc = 0;

    p_rvlc.conceal_max = CONCEAL_MAX_INIT as i16;
    p_rvlc.conceal_min = CONCEAL_MIN_INIT as i16;

    p_rvlc.conceal_max_esc = CONCEAL_MAX_INIT as i16;
    p_rvlc.conceal_min_esc = CONCEAL_MIN_INIT as i16;

    p_rvlc.p_huff_tree_rvlc_escape = &A_HUFF_TREE_RVLC_ESCAPE;
    p_rvlc.p_huff_tree_rvl_codewds = &A_HUFF_TREE_RVL_CODEWDS;

    /* init scf arrays (for savety (in case of there are only zero codebooks)) */
    for bnds in 0..RVLC_MAX_SFB as usize {
        wr(p_scf_fwd, bnds, 0);
        wr(p_scf_bwd, bnds, 0);
        wr(p_scf_esc, bnds, 0);
        p_dyn_data.a_scale_factor[bnds] = 0;
    }

    /* set base bitstream ptr to the RVL-coded part (start of RVLC data (ESC 2)) */
    fdk_sync_cache(bs);
    p_rvlc.bs_anchor = fdk_get_valid_bits(bs) as i32;

    /* first bit within RVL coded block as start address for forward decoding */
    p_rvlc.bitstream_index_rvl_fwd = 0;
    /* last bit within RVL coded block as start address for backward decoding */
    p_rvlc.bitstream_index_rvl_bwd = p_rvlc.length_of_rvlc_sf as i32 - 1;

    /* skip RVLC-bitstream-part -- pointing now to escapes (if present) or to TNS data (if
    present) */
    // FDKpushFor(bs, SHORT): a negative length (possible with noise, see CRvlc_Read()) becomes a
    // huge UINT, which moves the position back
    fdk_push_for(bs, p_rvlc.length_of_rvlc_sf as i32 as u32);

    if p_rvlc.sf_escapes_present != 0 {
        /* locate internal bitstream ptr at escapes (which is the second part) */
        fdk_sync_cache(bs);
        p_rvlc.bitstream_index_esc = p_rvlc.bs_anchor.wrapping_sub(fdk_get_valid_bits(bs) as i32);

        /* skip escapeRVLC-bitstream-part -- pointing to TNS data (if present) to make decoder
        continue */
        /* decoding of RVLC should work despite this second pushFor during initialization
        because bitstream initialization is valid for both ESC2 data parts (RVL-coded values and
        ESC-coded values) */
        fdk_push_for(bs, p_rvlc.length_of_rvlc_escapes as i32 as u32);
    }
}

/// `rvlcCheckIntensityCb(pRvlc, pAacDecoderChannelInfo)`: checks if an intensity codebook is
/// used in the current channel (`intensity_used`).
pub fn rvlc_check_intensity_cb(
    p_rvlc: &mut CErRvlcInfo,
    p_aac_decoder_channel_info: &CAacDecoderChannelInfo,
) {
    p_rvlc.intensity_used = 0;

    for group in 0..p_rvlc.num_window_groups as usize {
        for band in 0..p_rvlc.max_sfb_transmitted as usize {
            let bnds = 16 * group + band;
            let cb = rd(&p_aac_decoder_channel_info.p_dyn_data.a_code_book, bnds);
            if (cb == INTENSITY_HCB) || (cb == INTENSITY_HCB2) {
                p_rvlc.intensity_used = 1;
                break;
            }
        }
    }
}

/// `rvlcDecodeEscapeWord(pRvlc, bs)`: decodes a Huffman coded RVLC escape word (part of a DPCM
/// coded scale factor). Returns the escape value, or -1 on an error.
pub fn rvlc_decode_escape_word(p_rvlc: &mut CErRvlcInfo, bs: &mut FdkBitStream) -> i8 {
    let mut branch_value: u32 = 0;
    let mut branch_node: u32 = 0;

    let p_esc_tree = p_rvlc.p_huff_tree_rvlc_escape;
    let mut tree_node = rd(p_esc_tree, 0); /* init at starting node */

    let mut i = MAX_LEN_RVLC_ESCAPE_WORD - 1;
    while i >= 0 {
        /* get next bit */
        let carry_bit = rvlc_read_bit_from_bitstream(
            bs,
            p_rvlc.bs_anchor,
            &mut p_rvlc.bitstream_index_esc,
            FWD,
        );

        /* huffman decoding, do a single step in huffman decoding tree */
        carry_bit_to_branch_value(carry_bit, tree_node, &mut branch_value, &mut branch_node);

        if (branch_node & TEST_BIT_10 as u32) == TEST_BIT_10 as u32 {
            /* test bit 10 ; if set --> a RVLC-escape-word is completely decoded */
            // (SCHAR)branchNode & CLR_BIT_10, stored into an SCHAR
            let mut value: i8 = ((branch_node as i8 as i32) & CLR_BIT_10) as i8;
            p_rvlc.length_of_rvlc_escapes =
                (p_rvlc.length_of_rvlc_escapes as i32 - (MAX_LEN_RVLC_ESCAPE_WORD - i)) as i16;

            if p_rvlc.length_of_rvlc_escapes < 0 {
                p_rvlc.error_log_rvlc |= RVLC_ERROR_ALL_ESCAPE_WORDS_INVALID;
                value = -1;
            }

            return value;
        } else {
            /* update treeNode for further step in decoding tree */
            tree_node = rd(p_esc_tree, branch_value as usize);
        }
        i -= 1;
    }

    p_rvlc.error_log_rvlc |= RVLC_ERROR_ALL_ESCAPE_WORDS_INVALID;

    -1 /* should not be reached */
}

/// `rvlcDecodeEscapes(pRvlc, pEsc, bs)`: decodes all Huffman coded RVLC escape words (before the
/// RVL coded scale factors, which then take the escapes in ascending order).
pub fn rvlc_decode_escapes(p_rvlc: &mut CErRvlcInfo, p_esc: &mut [i16], bs: &mut FdkBitStream) {
    let mut esc_cnt: i8 = 0;

    /* Decode all RVLC-Escape words with a plain Huffman-Decoder */
    while p_rvlc.length_of_rvlc_escapes > 0 {
        let esc_word = rvlc_decode_escape_word(p_rvlc, bs);

        if esc_word >= 0 {
            wr(p_esc, esc_cnt as isize as usize, esc_word as i16);
            esc_cnt = esc_cnt.wrapping_add(1);
        } else {
            p_rvlc.error_log_rvlc |= RVLC_ERROR_ALL_ESCAPE_WORDS_INVALID;
            p_rvlc.num_decoded_escape_words_esc = esc_cnt as u8;

            return;
        }
    } /* all RVLC escapes decoded */

    p_rvlc.num_decoded_escape_words_esc = esc_cnt as u8;
}

/// `decodeRVLCodeword(bs, pRvlc)`: decodes an RVL coded DPCM word in the direction
/// `pRvlc->direction`. Returns a value in `0..=14` (the offset 7 must be subtracted), or -1 for
/// a forbidden codeword or too many decoded bits.
pub fn decode_rvl_codeword(bs: &mut FdkBitStream, p_rvlc: &mut CErRvlcInfo) -> i8 {
    let mut branch_value: u32 = 0;
    let mut branch_node: u32 = 0;

    let p_rvl_code_tree = p_rvlc.p_huff_tree_rvl_codewds;
    let direction = p_rvlc.direction;
    let mut tree_node = rd(p_rvl_code_tree, 0);

    let mut i = MAX_LEN_RVLC_CODE_WORD - 1;
    while i >= 0 {
        /* get next bit */
        let bs_anchor = p_rvlc.bs_anchor;
        let carry_bit =
            rvlc_read_bit_from_bitstream(bs, bs_anchor, p_rvlc.bitstr_indx_rvl_rvl(), direction);

        /* huffman decoding, do a single step in huffman decoding tree */
        carry_bit_to_branch_value(carry_bit, tree_node, &mut branch_value, &mut branch_node);

        if (branch_node & TEST_BIT_10 as u32) == TEST_BIT_10 as u32 {
            /* test bit 10 ; if set --> a RVLC-codeword is completely decoded */
            let mut value: i8 = (branch_node & CLR_BIT_10 as u32) as i8;
            let cnt = p_rvlc.rvl_bit_cnt_rvl();
            *cnt = (*cnt as i32 - (MAX_LEN_RVLC_CODE_WORD - i)) as i16;

            /* check available bits for decoding */
            if *p_rvlc.rvl_bit_cnt_rvl() < 0 {
                if direction == FWD {
                    p_rvlc.error_log_rvlc |= RVLC_ERROR_RVL_SUM_BIT_COUNTER_BELOW_ZERO_FWD;
                } else {
                    p_rvlc.error_log_rvlc |= RVLC_ERROR_RVL_SUM_BIT_COUNTER_BELOW_ZERO_BWD;
                }
                /* signalize an error in return value, because too many bits was decoded */
                value = -1;
            }

            /* check max value of dpcm value */
            if value as i32 > MAX_ALLOWED_DPCM_INDEX {
                if direction == FWD {
                    p_rvlc.error_log_rvlc |= RVLC_ERROR_FORBIDDEN_CW_DETECTED_FWD;
                } else {
                    p_rvlc.error_log_rvlc |= RVLC_ERROR_FORBIDDEN_CW_DETECTED_BWD;
                }
                /* signalize an error in return value, because a forbidden cw was detected */
                value = -1;
            }

            return value; /* return a dpcm value with offset +7 or an error status */
        } else {
            /* update treeNode for further step in decoding tree */
            tree_node = rd(p_rvl_code_tree, branch_value as usize);
        }
        i -= 1;
    }

    -1
}

/// The escape handling of `rvlcDecodeForward()` after `dpcm -= TABLE_OFFSET`. Returns `true`
/// where C returns (`conceal_max` is set).
fn fwd_escape(
    p_rvlc: &mut CErRvlcInfo,
    dpcm: &mut i16,
    p_scf_esc: &[i16],
    esc_idx: &mut usize,
    bnds: i32,
) -> bool {
    if (*dpcm as i32 == MIN_RVL) || (*dpcm as i32 == MAX_RVL) {
        if p_rvlc.length_of_rvlc_escapes != 0 {
            p_rvlc.conceal_max = bnds as i16;
            return true;
        } else {
            let esc = rd(p_scf_esc, *esc_idx) as i32;
            *esc_idx += 1;
            if *dpcm as i32 == MIN_RVL {
                *dpcm = (*dpcm as i32 - esc) as i16;
            } else {
                *dpcm = (*dpcm as i32 + esc) as i16;
            }
            p_rvlc.num_decoded_escape_words_fwd =
                p_rvlc.num_decoded_escape_words_fwd.wrapping_add(1);
            if p_rvlc.conceal_max_esc as i32 == CONCEAL_MAX_INIT {
                p_rvlc.conceal_max_esc = bnds as i16;
            }
        }
    }
    false
}

/// `rvlcDecodeForward(pRvlc, pAacDecoderChannelInfo, bs)`: decodes the RVL coded codewords in
/// forward direction.
///
/// `p_scf_fwd` and `p_scf_esc` are `pComData->overlay.aac.aRvlcScfFwd` and `aRvlcScfEsc`.
pub fn rvlc_decode_forward(
    p_rvlc: &mut CErRvlcInfo,
    p_aac_decoder_channel_info: &mut CAacDecoderChannelInfo,
    bs: &mut FdkBitStream,
    p_scf_fwd: &mut [i16],
    p_scf_esc: &[i16],
) {
    let mut bnds: i32 = 0;
    let mut dpcm: i16;

    let global_gain = p_aac_decoder_channel_info.p_dyn_data.raw_data_info.global_gain as i32;
    let mut factor: i16 = (global_gain - SF_OFFSET) as i16;
    let mut position: i16 = (-SF_OFFSET) as i16;
    let mut noisenrg: i16 = (global_gain - SF_OFFSET - 90 - 256) as i16;

    let mut esc_idx = 0usize; /* pScfEsc */

    p_rvlc.p_rvl_bit_cnt_rvl = FWD; /* &(pRvlc->length_of_rvlc_sf_fwd) */
    p_rvlc.p_bitstr_indx_rvl_rvl = FWD; /* &(pRvlc->bitstreamIndexRvlFwd) */

    p_rvlc.num_decoded_escape_words_fwd = 0;
    p_rvlc.direction = FWD;
    p_rvlc.noise_used = 0;
    p_rvlc.sf_used = 0;
    p_rvlc.last_scf = 0;
    p_rvlc.last_nrg = 0;
    p_rvlc.last_is = 0;

    rvlc_check_intensity_cb(p_rvlc, p_aac_decoder_channel_info);

    /* main loop fwd long */
    for group in 0..p_rvlc.num_window_groups as i32 {
        for band in 0..p_rvlc.max_sfb_transmitted as i32 {
            bnds = 16 * group + band;

            match rd(&p_aac_decoder_channel_info.p_dyn_data.a_code_book, bnds as usize) {
                ZERO_HCB => {
                    wr(p_scf_fwd, bnds as usize, 0);
                }

                INTENSITY_HCB2 | INTENSITY_HCB => {
                    /* store dpcm_is_position */
                    dpcm = decode_rvl_codeword(bs, p_rvlc) as i16;
                    if dpcm < 0 {
                        p_rvlc.conceal_max = bnds as i16;
                        return;
                    }
                    dpcm -= TABLE_OFFSET as i16;
                    if fwd_escape(p_rvlc, &mut dpcm, p_scf_esc, &mut esc_idx, bnds) {
                        return;
                    }
                    position = (position as i32 + dpcm as i32) as i16;
                    wr(p_scf_fwd, bnds as usize, position);
                    p_rvlc.last_is = position;
                }

                NOISE_HCB => {
                    if p_rvlc.noise_used == 0 {
                        p_rvlc.noise_used = 1;
                        p_rvlc.first_noise_band = bnds as u8;
                        noisenrg = (noisenrg as i32 + p_rvlc.dpcm_noise_nrg) as i16;
                        wr(p_scf_fwd, bnds as usize, (100 + noisenrg as i32) as i16);
                        p_rvlc.last_nrg = noisenrg;
                    } else {
                        dpcm = decode_rvl_codeword(bs, p_rvlc) as i16;
                        if dpcm < 0 {
                            p_rvlc.conceal_max = bnds as i16;
                            return;
                        }
                        dpcm -= TABLE_OFFSET as i16;
                        if fwd_escape(p_rvlc, &mut dpcm, p_scf_esc, &mut esc_idx, bnds) {
                            return;
                        }
                        noisenrg = (noisenrg as i32 + dpcm as i32) as i16;
                        wr(p_scf_fwd, bnds as usize, (100 + noisenrg as i32) as i16);
                        p_rvlc.last_nrg = noisenrg;
                    }
                    wr(
                        &mut p_aac_decoder_channel_info.data.aac.pns_data.pns_used,
                        bnds as usize,
                        1,
                    );
                }

                _ => {
                    p_rvlc.sf_used = 1;
                    dpcm = decode_rvl_codeword(bs, p_rvlc) as i16;
                    if dpcm < 0 {
                        p_rvlc.conceal_max = bnds as i16;
                        return;
                    }
                    dpcm -= TABLE_OFFSET as i16;
                    if fwd_escape(p_rvlc, &mut dpcm, p_scf_esc, &mut esc_idx, bnds) {
                        return;
                    }
                    factor = (factor as i32 + dpcm as i32) as i16;
                    wr(p_scf_fwd, bnds as usize, factor);
                    p_rvlc.last_scf = factor;
                }
            }
        }
    }

    /* postfetch fwd long */
    if p_rvlc.intensity_used != 0 {
        dpcm = decode_rvl_codeword(bs, p_rvlc) as i16; /* dpcm_is_last_position */
        if dpcm < 0 {
            p_rvlc.conceal_max = bnds as i16;
            return;
        }
        dpcm -= TABLE_OFFSET as i16;
        if fwd_escape(p_rvlc, &mut dpcm, p_scf_esc, &mut esc_idx, bnds) {
            return;
        }
        p_rvlc.dpcm_is_last_position = dpcm as i32;
    }
}

/// The escape handling of `rvlcDecodeBackward()` after `dpcm -= TABLE_OFFSET`. Returns `true`
/// where C returns (the caller sets `conceal_min`); `conceal_min_esc` is the value that
/// `conceal_min_esc` gets if it is still at its initial value.
fn bwd_escape(
    p_rvlc: &mut CErRvlcInfo,
    dpcm: &mut i16,
    p_scf_esc: &[i16],
    esc_idx: &mut i32,
    esc_esc_cnt: u8,
    conceal_min_esc: i16,
) -> bool {
    if (*dpcm as i32 == MIN_RVL) || (*dpcm as i32 == MAX_RVL) {
        if (p_rvlc.length_of_rvlc_escapes != 0)
            || (p_rvlc.num_decoded_escape_words_bwd >= esc_esc_cnt)
        {
            return true;
        } else {
            let esc = rd(p_scf_esc, *esc_idx as usize) as i32;
            *esc_idx -= 1;
            if *dpcm as i32 == MIN_RVL {
                *dpcm = (*dpcm as i32 - esc) as i16;
            } else {
                *dpcm = (*dpcm as i32 + esc) as i16;
            }
            p_rvlc.num_decoded_escape_words_bwd =
                p_rvlc.num_decoded_escape_words_bwd.wrapping_add(1);
            if p_rvlc.conceal_min_esc as i32 == CONCEAL_MIN_INIT {
                p_rvlc.conceal_min_esc = conceal_min_esc;
            }
        }
    }
    false
}

/// `rvlcDecodeBackward(pRvlc, pAacDecoderChannelInfo, bs)`: decodes the RVL coded codewords in
/// backward direction.
///
/// `p_scf_bwd` and `p_scf_esc` are `pComData->overlay.aac.aRvlcScfBwd` and `aRvlcScfEsc`.
pub fn rvlc_decode_backward(
    p_rvlc: &mut CErRvlcInfo,
    p_aac_decoder_channel_info: &mut CAacDecoderChannelInfo,
    bs: &mut FdkBitStream,
    p_scf_bwd: &mut [i16],
    p_scf_esc: &[i16],
) {
    let mut dpcm: i16;
    let mut offset: i16;
    let mut bnds: i16 = (p_rvlc.max_sfb_transmitted as i32 - 1) as i16;

    let mut factor: i16 = (p_rvlc.rev_global_gain - SF_OFFSET) as i16;
    let mut position: i16 = (p_rvlc.dpcm_is_last_position - SF_OFFSET) as i16;
    let mut noisenrg: i16 =
        (p_rvlc.rev_global_gain + p_rvlc.dpcm_noise_last_position - SF_OFFSET - 90 - 256) as i16;

    let esc_esc_cnt: u8 = p_rvlc.num_decoded_escape_words_esc;

    p_rvlc.p_rvl_bit_cnt_rvl = BWD; /* &(pRvlc->length_of_rvlc_sf_bwd) */
    p_rvlc.p_bitstr_indx_rvl_rvl = BWD; /* &(pRvlc->bitstreamIndexRvlBwd) */

    p_rvlc.num_decoded_escape_words_bwd = 0;
    p_rvlc.direction = BWD;
    let mut esc_idx: i32 = esc_esc_cnt as i32 - 1; /* set pScfEsc to last entry */
    p_rvlc.first_scf = 0;
    p_rvlc.first_nrg = 0;
    p_rvlc.first_is = 0;

    /* prefetch long BWD */
    if p_rvlc.intensity_used != 0 {
        dpcm = decode_rvl_codeword(bs, p_rvlc) as i16; /* dpcm_is_last_position */
        if dpcm < 0 {
            p_rvlc.dpcm_is_last_position = 0;
            p_rvlc.conceal_min = bnds;
            return;
        }
        dpcm -= TABLE_OFFSET as i16;
        if bwd_escape(p_rvlc, &mut dpcm, p_scf_esc, &mut esc_idx, esc_esc_cnt, bnds) {
            p_rvlc.conceal_min = bnds;
            return;
        }
        p_rvlc.dpcm_is_last_position = dpcm as i32;
    }

    /* main loop long BWD */
    let mut group: i16 = (p_rvlc.num_window_groups as i32 - 1) as i16;
    while group >= 0 {
        let mut band: i16 = (p_rvlc.max_sfb_transmitted as i32 - 1) as i16;
        while band >= 0 {
            bnds = (16 * group as i32 + band as i32) as i16;
            if (band == 0) && (p_rvlc.num_window_groups != 1) {
                offset = (16 - p_rvlc.max_sfb_transmitted as i32 + 1) as i16;
            } else {
                offset = 1;
            }
            let conceal = f_max(0, bnds as i32 - offset as i32) as i16;

            match rd(&p_aac_decoder_channel_info.p_dyn_data.a_code_book, bnds as usize) {
                ZERO_HCB => {
                    wr(p_scf_bwd, bnds as usize, 0);
                }

                INTENSITY_HCB2 | INTENSITY_HCB => {
                    /* store dpcm_is_position */
                    dpcm = decode_rvl_codeword(bs, p_rvlc) as i16;
                    if dpcm < 0 {
                        wr(p_scf_bwd, bnds as usize, position);
                        p_rvlc.conceal_min = conceal;
                        return;
                    }
                    dpcm -= TABLE_OFFSET as i16;
                    if bwd_escape(p_rvlc, &mut dpcm, p_scf_esc, &mut esc_idx, esc_esc_cnt, conceal)
                    {
                        wr(p_scf_bwd, bnds as usize, position);
                        p_rvlc.conceal_min = conceal;
                        return;
                    }
                    wr(p_scf_bwd, bnds as usize, position);
                    position = (position as i32 - dpcm as i32) as i16;
                    p_rvlc.first_is = position;
                }

                NOISE_HCB => {
                    if bnds as i32 == p_rvlc.first_noise_band as i32 {
                        let v = (p_rvlc.dpcm_noise_nrg
                            + p_aac_decoder_channel_info.p_dyn_data.raw_data_info.global_gain
                                as i32
                            - SF_OFFSET
                            - 90
                            - 256) as i16;
                        wr(p_scf_bwd, bnds as usize, v);
                        p_rvlc.first_nrg = rd(p_scf_bwd, bnds as usize);
                    } else {
                        dpcm = decode_rvl_codeword(bs, p_rvlc) as i16;
                        if dpcm < 0 {
                            wr(p_scf_bwd, bnds as usize, noisenrg);
                            p_rvlc.conceal_min = conceal;
                            return;
                        }
                        dpcm -= TABLE_OFFSET as i16;
                        if bwd_escape(
                            p_rvlc,
                            &mut dpcm,
                            p_scf_esc,
                            &mut esc_idx,
                            esc_esc_cnt,
                            conceal,
                        ) {
                            wr(p_scf_bwd, bnds as usize, noisenrg);
                            p_rvlc.conceal_min = conceal;
                            return;
                        }
                        wr(p_scf_bwd, bnds as usize, noisenrg);
                        noisenrg = (noisenrg as i32 - dpcm as i32) as i16;
                        p_rvlc.first_nrg = noisenrg;
                    }
                }

                _ => {
                    dpcm = decode_rvl_codeword(bs, p_rvlc) as i16;
                    if dpcm < 0 {
                        wr(p_scf_bwd, bnds as usize, factor);
                        p_rvlc.conceal_min = conceal;
                        return;
                    }
                    dpcm -= TABLE_OFFSET as i16;
                    if bwd_escape(p_rvlc, &mut dpcm, p_scf_esc, &mut esc_idx, esc_esc_cnt, conceal)
                    {
                        wr(p_scf_bwd, bnds as usize, factor);
                        p_rvlc.conceal_min = conceal;
                        return;
                    }
                    wr(p_scf_bwd, bnds as usize, factor);
                    factor = (factor as i32 - dpcm as i32) as i16;
                    p_rvlc.first_scf = factor;
                }
            }
            band -= 1;
        }
        group -= 1;
    }
}

/// `rvlcFinalErrorDetection(pAacDecoderChannelInfo, pAacDecoderStaticChannelInfo)`: calls the
/// RVLC concealment if an error was detected in the decoding process.
pub fn rvlc_final_error_detection(
    p_aac_decoder_channel_info: &mut CAacDecoderChannelInfo,
    p_aac_decoder_static_channel_info: &mut CAacDecoderStaticChannelInfo,
    com: &mut CAacDecoderCommonData,
) {
    let mut error_status_length_fwd: u8 = 0;
    let mut error_status_length_bwd: u8 = 0;
    let mut error_status_length_escapes: u8 = 0;
    let mut error_status_first_scf: u8 = 0;
    let mut error_status_last_scf: u8 = 0;
    let mut error_status_first_nrg: u8 = 0;
    let mut error_status_last_nrg: u8 = 0;
    let mut error_status_first_is: u8 = 0;
    let mut error_status_last_is: u8 = 0;
    let mut error_status_forbidden_cw_fwd: u8 = 0;
    let mut error_status_forbidden_cw_bwd: u8 = 0;
    let mut error_status_num_escapes_fwd: u8 = 0;
    let mut error_status_num_escapes_bwd: u8 = 0;
    let mut conceal_status: u8 = 1;

    p_aac_decoder_channel_info.p_dyn_data.specific_to.aac.rvlc_current_scale_factor_ok = 1;

    let global_gain = p_aac_decoder_channel_info.p_dyn_data.raw_data_info.global_gain as i32;
    let error_status_complete: bool;
    {
        let p_rvlc = &mut com.overlay.aac.er_rvlc_info;

        /* invalid escape words, bit counter unequal zero, forbidden codeword detected */
        if p_rvlc.error_log_rvlc & RVLC_ERROR_FORBIDDEN_CW_DETECTED_FWD != 0 {
            error_status_forbidden_cw_fwd = 1;
        }

        if p_rvlc.error_log_rvlc & RVLC_ERROR_FORBIDDEN_CW_DETECTED_BWD != 0 {
            error_status_forbidden_cw_bwd = 1;
        }

        /* bit counter forward unequal zero */
        if p_rvlc.length_of_rvlc_sf_fwd != 0 {
            error_status_length_fwd = 1;
        }

        /* bit counter backward unequal zero */
        if p_rvlc.length_of_rvlc_sf_bwd != 0 {
            error_status_length_bwd = 1;
        }

        /* bit counter escape sequences unequal zero */
        if p_rvlc.sf_escapes_present != 0 {
            if p_rvlc.length_of_rvlc_escapes != 0 {
                error_status_length_escapes = 1;
            }
        }

        if p_rvlc.sf_used != 0 {
            /* first decoded scf does not match to global gain in backward direction */
            if p_rvlc.first_scf as i32 != (global_gain - SF_OFFSET) {
                error_status_first_scf = 1;
            }

            /* last decoded scf does not match to rev global gain in forward direction */
            if p_rvlc.last_scf as i32 != (p_rvlc.rev_global_gain - SF_OFFSET) {
                error_status_last_scf = 1;
            }
        }

        if p_rvlc.noise_used != 0 {
            /* first decoded nrg does not match to dpcm_noise_nrg in backward direction */
            if p_rvlc.first_nrg as i32
                != (global_gain + p_rvlc.dpcm_noise_nrg - SF_OFFSET - 90 - 256)
            {
                error_status_first_nrg = 1;
            }

            /* last decoded nrg does not match to dpcm_noise_last_position in forward direction */
            if p_rvlc.last_nrg as i32
                != (p_rvlc.rev_global_gain + p_rvlc.dpcm_noise_last_position - SF_OFFSET - 90 - 256)
            {
                error_status_last_nrg = 1;
            }
        }

        if p_rvlc.intensity_used != 0 {
            /* first decoded is position does not match in backward direction */
            if p_rvlc.first_is as i32 != (-SF_OFFSET) {
                error_status_first_is = 1;
            }

            /* last decoded is position does not match in forward direction */
            if p_rvlc.last_is as i32 != (p_rvlc.dpcm_is_last_position - SF_OFFSET) {
                error_status_last_is = 1;
            }
        }

        /* decoded escapes and used escapes in forward direction do not fit */
        if (p_rvlc.num_decoded_escape_words_fwd != p_rvlc.num_decoded_escape_words_esc)
            && (p_rvlc.conceal_max as i32 == CONCEAL_MAX_INIT)
        {
            error_status_num_escapes_fwd = 1;
        }

        /* decoded escapes and used escapes in backward direction do not fit */
        if (p_rvlc.num_decoded_escape_words_bwd != p_rvlc.num_decoded_escape_words_esc)
            && (p_rvlc.conceal_min as i32 == CONCEAL_MIN_INIT)
        {
            error_status_num_escapes_bwd = 1;
        }

        if error_status_length_escapes != 0
            || (((p_rvlc.conceal_max as i32 == CONCEAL_MAX_INIT)
                && (p_rvlc.num_decoded_escape_words_fwd != p_rvlc.num_decoded_escape_words_esc)
                && (error_status_last_scf != 0
                    || error_status_last_nrg != 0
                    || error_status_last_is != 0))
                && ((p_rvlc.conceal_min as i32 == CONCEAL_MIN_INIT)
                    && (p_rvlc.num_decoded_escape_words_bwd
                        != p_rvlc.num_decoded_escape_words_esc)
                    && (error_status_first_scf != 0
                        || error_status_first_nrg != 0
                        || error_status_first_is != 0)))
            || ((p_rvlc.conceal_max as i32 == CONCEAL_MAX_INIT)
                && ((p_rvlc.rev_global_gain - SF_OFFSET - p_rvlc.last_scf as i32) < -15))
            || ((p_rvlc.conceal_min as i32 == CONCEAL_MIN_INIT)
                && ((global_gain - SF_OFFSET - p_rvlc.first_scf as i32) < -15))
        {
            if (p_rvlc.conceal_max as i32 == CONCEAL_MAX_INIT)
                || (p_rvlc.conceal_min as i32 == CONCEAL_MIN_INIT)
            {
                p_rvlc.conceal_max = 0;
                p_rvlc.conceal_min = f_max(
                    0,
                    (p_rvlc.num_window_groups as i32 - 1) * 16 + p_rvlc.max_sfb_transmitted as i32
                        - 1,
                ) as i16;
            } else {
                p_rvlc.conceal_max = f_min(p_rvlc.conceal_max, p_rvlc.conceal_max_esc);
                p_rvlc.conceal_min = f_max(p_rvlc.conceal_min, p_rvlc.conceal_min_esc);
            }
        }

        error_status_complete = error_status_last_scf != 0
            || error_status_first_scf != 0
            || error_status_last_nrg != 0
            || error_status_first_nrg != 0
            || error_status_last_is != 0
            || error_status_first_is != 0
            || error_status_forbidden_cw_fwd != 0
            || error_status_forbidden_cw_bwd != 0
            || error_status_length_fwd != 0
            || error_status_length_bwd != 0
            || error_status_length_escapes != 0
            || error_status_num_escapes_fwd != 0
            || error_status_num_escapes_bwd != 0;
    }

    /* short: 0, not short: 1*/
    let current_block_type: u8 =
        if get_window_sequence(&p_aac_decoder_channel_info.ics_info) == BLOCK_SHORT {
            0
        } else {
            1
        };

    if !error_status_complete {
        let aac = &com.overlay.aac;
        let p_rvlc = &aac.er_rvlc_info;
        let ci = &mut p_aac_decoder_static_channel_info.concealment_info;
        let p_dyn_data = &mut *p_aac_decoder_channel_info.p_dyn_data;

        let last_sfb_index: usize = if p_rvlc.num_window_groups > 1 { 16 } else { 64 };

        for group in 0..p_rvlc.num_window_groups as usize {
            for band in 0..p_rvlc.max_sfb_transmitted as usize {
                let bnds = 16 * group + band;
                let v = rd(&aac.a_rvlc_scf_fwd, bnds);
                wr(&mut ci.a_rvlc_previous_scale_factor, bnds, v);
                wr(&mut p_dyn_data.a_scale_factor, bnds, v);
            }
        }

        for group in 0..p_rvlc.num_window_groups as usize {
            let mut band = 0usize;
            while band < p_rvlc.max_sfb_transmitted as usize {
                let bnds = 16 * group + band;
                wr(&mut ci.a_rvlc_previous_codebook, bnds, rd(&p_dyn_data.a_code_book, bnds));
                band += 1;
            }
            while band < last_sfb_index {
                let bnds = 16 * group + band;
                fdk_assert!(bnds < RVLC_MAX_SFB as usize);
                wr(&mut ci.a_rvlc_previous_codebook, bnds, ZERO_HCB);
                band += 1;
            }
        }
    } else {
        /* A single bit error was detected in decoding of dpcm values. It also could be an error
        with more bits in decoding of escapes and dpcm values whereby an illegal codeword
        followed not directly after the corrupted bits but just after decoding some more (wrong)
        scalefactors. Use the smaller scalefactor from forward decoding, backward decoding and
        previous frame. */
        {
            let p_rvlc = &com.overlay.aac.er_rvlc_info;
            let ci = &p_aac_decoder_static_channel_info.concealment_info;
            if ((p_rvlc.conceal_min as i32 != CONCEAL_MIN_INIT)
                || (p_rvlc.conceal_max as i32 != CONCEAL_MAX_INIT))
                && (p_rvlc.conceal_min <= p_rvlc.conceal_max)
                && (ci.rvlc_previous_block_type as i32 == current_block_type as i32)
                && ci.rvlc_previous_scale_factor_ok != 0
                && p_rvlc.sf_concealment != 0
                && conceal_status != 0
            {
                bidirectional_estimation_use_scf_of_prev_frame_as_reference(
                    p_aac_decoder_channel_info,
                    p_aac_decoder_static_channel_info,
                    com,
                );
                conceal_status = 0;
            }
        }

        /* A single bit error was detected in decoding of dpcm values. It also could be an error
        with more bits in decoding of escapes and dpcm values whereby an illegal codeword
        followed not directly after the corrupted bits but just after decoding some more (wrong)
        scalefactors. Use the smaller scalefactor from forward and backward decoding. */
        {
            let p_rvlc = &com.overlay.aac.er_rvlc_info;
            let ci = &p_aac_decoder_static_channel_info.concealment_info;
            if (p_rvlc.conceal_min <= p_rvlc.conceal_max)
                && ((p_rvlc.conceal_min as i32 != CONCEAL_MIN_INIT)
                    || (p_rvlc.conceal_max as i32 != CONCEAL_MAX_INIT))
                && !(ci.rvlc_previous_scale_factor_ok != 0
                    && p_rvlc.sf_concealment != 0
                    && (ci.rvlc_previous_block_type as i32 == current_block_type as i32))
                && conceal_status != 0
            {
                bidirectional_estimation_use_lower_scf_of_current_frame(
                    p_aac_decoder_channel_info,
                    com,
                );
                conceal_status = 0;
            }
        }

        /* No errors were detected in decoding of escapes and dpcm values however the first and
        last value of a group (is,nrg,sf) is incorrect */
        {
            let p_rvlc = &com.overlay.aac.er_rvlc_info;
            if (p_rvlc.conceal_min <= p_rvlc.conceal_max)
                && ((error_status_last_scf != 0 && error_status_first_scf != 0)
                    || (error_status_last_nrg != 0 && error_status_first_nrg != 0)
                    || (error_status_last_is != 0 && error_status_first_is != 0))
                && !(error_status_forbidden_cw_fwd != 0
                    || error_status_forbidden_cw_bwd != 0
                    || error_status_length_escapes != 0)
                && conceal_status != 0
            {
                statistical_estimation(p_aac_decoder_channel_info, com);
                conceal_status = 0;
            }
        }

        /* A error with more bits in decoding of escapes and dpcm values was detected. Use the
        smaller scalefactor from forward decoding, backward decoding and previous frame. */
        {
            let p_rvlc = &com.overlay.aac.er_rvlc_info;
            let ci = &p_aac_decoder_static_channel_info.concealment_info;
            if (p_rvlc.conceal_min <= p_rvlc.conceal_max)
                && ci.rvlc_previous_scale_factor_ok != 0
                && p_rvlc.sf_concealment != 0
                && (ci.rvlc_previous_block_type as i32 == current_block_type as i32)
                && conceal_status != 0
            {
                predictive_interpolation(
                    p_aac_decoder_channel_info,
                    p_aac_decoder_static_channel_info,
                    com,
                );
                conceal_status = 0;
            }
        }

        /* Call frame concealment, because no better strategy was found. Setting the scalefactors
        to zero is done for debugging purposes */
        if conceal_status != 0 {
            let p_rvlc = &com.overlay.aac.er_rvlc_info;
            let p_dyn_data = &mut *p_aac_decoder_channel_info.p_dyn_data;
            for group in 0..p_rvlc.num_window_groups as usize {
                for band in 0..p_rvlc.max_sfb_transmitted as usize {
                    wr(&mut p_dyn_data.a_scale_factor, 16 * group + band, 0);
                }
            }
            p_dyn_data.specific_to.aac.rvlc_current_scale_factor_ok = 0;
        }
    }
}

/// `CRvlc_Read(pAacDecoderChannelInfo, bs)`: reads the RVLC ESC1 data (side info) from the
/// bitstream.
///
/// `com` is the element's `pComData` (holds `erRvlcInfo`).
pub fn crvlc_read(
    p_aac_decoder_channel_info: &mut CAacDecoderChannelInfo,
    bs: &mut FdkBitStream,
    com: &mut CAacDecoderCommonData,
) {
    let p_rvlc = &mut com.overlay.aac.er_rvlc_info;

    /* RVLC long specific initialization  Init part 1 of 2 */
    p_rvlc.num_window_groups = get_window_groups(&p_aac_decoder_channel_info.ics_info);
    p_rvlc.max_sfb_transmitted =
        get_scale_factor_bands_transmitted(&p_aac_decoder_channel_info.ics_info);
    p_rvlc.noise_used = 0; /* noise detection */
    p_rvlc.dpcm_noise_nrg = 0; /* only for debugging */
    p_rvlc.dpcm_noise_last_position = 0; /* only for debugging */
    p_rvlc.length_of_rvlc_escapes = -1; /* default value is used for error detection and
                                        concealment */

    /* read only error sensitivity class 1 data (ESC 1 - data) */
    p_rvlc.sf_concealment = fdk_read_bits(bs, 1) as i32; /* #1 */
    p_rvlc.rev_global_gain = fdk_read_bits(bs, 8) as i32; /* #2 */

    if get_window_sequence(&p_aac_decoder_channel_info.ics_info) == BLOCK_SHORT {
        p_rvlc.length_of_rvlc_sf = fdk_read_bits(bs, 11) as i16; /* #3 */
    } else {
        p_rvlc.length_of_rvlc_sf = fdk_read_bits(bs, 9) as i16; /* #3 */
    }

    /* check if noise codebook is used */
    for group in 0..p_rvlc.num_window_groups as usize {
        for band in 0..p_rvlc.max_sfb_transmitted as usize {
            if rd(&p_aac_decoder_channel_info.p_dyn_data.a_code_book, 16 * group + band)
                == NOISE_HCB
            {
                p_rvlc.noise_used = 1;
                break;
            }
        }
    }

    if p_rvlc.noise_used != 0 {
        p_rvlc.dpcm_noise_nrg = fdk_read_bits(bs, 9) as i32; /* #4  PNS */
    }

    p_rvlc.sf_escapes_present = fdk_read_bits(bs, 1) as i32; /* #5      */

    if p_rvlc.sf_escapes_present != 0 {
        p_rvlc.length_of_rvlc_escapes = fdk_read_bits(bs, 8) as i16; /* #6      */
    }

    if p_rvlc.noise_used != 0 {
        p_rvlc.dpcm_noise_last_position = fdk_read_bits(bs, 9) as i32; /* #7  PNS */
        p_rvlc.length_of_rvlc_sf = (p_rvlc.length_of_rvlc_sf as i32 - 9) as i16;
    }

    p_rvlc.length_of_rvlc_sf_fwd = p_rvlc.length_of_rvlc_sf;
    p_rvlc.length_of_rvlc_sf_bwd = p_rvlc.length_of_rvlc_sf;
}

/// `CRvlc_Decode(pAacDecoderChannelInfo, pAacDecoderStaticChannelInfo, bs)`: decodes the RVLC
/// data: the escape sequences and the scale factors in forward and backward direction. If an
/// error that the RVLC concealment cannot conceal occurred, `rvlcCurrentScaleFactorOK` is set to
/// 0 (frame concealment), otherwise to 1.
///
/// `com` is the element's `pComData`.
pub fn crvlc_decode(
    p_aac_decoder_channel_info: &mut CAacDecoderChannelInfo,
    p_aac_decoder_static_channel_info: &mut CAacDecoderStaticChannelInfo,
    bs: &mut FdkBitStream,
    com: &mut CAacDecoderCommonData,
) {
    {
        let aac = &mut com.overlay.aac;
        rvlc_init(
            &mut aac.er_rvlc_info,
            p_aac_decoder_channel_info,
            bs,
            &mut aac.a_rvlc_scf_esc,
            &mut aac.a_rvlc_scf_fwd,
            &mut aac.a_rvlc_scf_bwd,
        );
    }

    /* save bitstream position */
    let save_bit_cnt = fdk_get_valid_bits(bs) as i32;

    {
        let aac = &mut com.overlay.aac;
        if aac.er_rvlc_info.sf_escapes_present != 0 {
            rvlc_decode_escapes(&mut aac.er_rvlc_info, &mut aac.a_rvlc_scf_esc, bs);
        }

        rvlc_decode_forward(
            &mut aac.er_rvlc_info,
            p_aac_decoder_channel_info,
            bs,
            &mut aac.a_rvlc_scf_fwd,
            &aac.a_rvlc_scf_esc,
        );
        rvlc_decode_backward(
            &mut aac.er_rvlc_info,
            p_aac_decoder_channel_info,
            bs,
            &mut aac.a_rvlc_scf_bwd,
            &aac.a_rvlc_scf_esc,
        );
    }
    rvlc_final_error_detection(p_aac_decoder_channel_info, p_aac_decoder_static_channel_info, com);

    let p_rvlc = &com.overlay.aac.er_rvlc_info;
    p_aac_decoder_channel_info.p_dyn_data.specific_to.aac.rvlc_intensity_used =
        p_rvlc.intensity_used;
    p_aac_decoder_channel_info.data.aac.pns_data.pns_active = p_rvlc.noise_used as u8;

    /* restore bitstream position */
    let bit_cnt_offst = (fdk_get_valid_bits(bs) as i32).wrapping_sub(save_bit_cnt);
    if bit_cnt_offst != 0 {
        fdk_push_bi_directional(bs, bit_cnt_offst);
    }
}

/// `CRvlc_ElementCheck(pAacDecoderChannelInfo, pAacDecoderStaticChannelInfo, flags,
/// elChannels)`: performs sanity checks on the channel data of one channel element.
///
/// `p_aac_decoder_static_channel_info` is `None` for `NULL` (MPS residuals). `com` is the
/// element's `pComData` (`pAacDecoderChannelInfo[0]->pComData`).
pub fn crvlc_element_check(
    p_aac_decoder_channel_info: &mut [&mut CAacDecoderChannelInfo],
    p_aac_decoder_static_channel_info: Option<&mut [&mut CAacDecoderStaticChannelInfo]>,
    flags: u32,
    el_channels: i32,
    com: &CAacDecoderCommonData,
) {
    /* Required for MPS residuals. */
    let Some(p_aac_decoder_static_channel_info) = p_aac_decoder_static_channel_info else {
        return;
    };

    /* RVLC specific sanity checks */
    if (flags & AC_ER_RVLC != 0) && (el_channels == 2) && p_aac_decoder_channel_info.len() >= 2 {
        /* to be reviewed */
        let ok0 =
            p_aac_decoder_channel_info[0].p_dyn_data.specific_to.aac.rvlc_current_scale_factor_ok;
        let ok1 =
            p_aac_decoder_channel_info[1].p_dyn_data.specific_to.aac.rvlc_current_scale_factor_ok;
        if ((ok0 == 0) || (ok1 == 0)) && com.joint_stereo_data.ms_mask_present != 0 {
            p_aac_decoder_channel_info[0].p_dyn_data.specific_to.aac.rvlc_current_scale_factor_ok =
                0;
            p_aac_decoder_channel_info[1].p_dyn_data.specific_to.aac.rvlc_current_scale_factor_ok =
                0;
        }

        if (p_aac_decoder_channel_info[0].p_dyn_data.specific_to.aac.rvlc_current_scale_factor_ok
            == 0)
            && (p_aac_decoder_channel_info[1]
                .p_dyn_data
                .specific_to
                .aac
                .rvlc_current_scale_factor_ok
                == 1)
            && (p_aac_decoder_channel_info[1].p_dyn_data.specific_to.aac.rvlc_intensity_used == 1)
        {
            p_aac_decoder_channel_info[1].p_dyn_data.specific_to.aac.rvlc_current_scale_factor_ok =
                0;
        }
    }

    for (ch, st) in p_aac_decoder_channel_info
        .iter()
        .zip(p_aac_decoder_static_channel_info.iter_mut())
        .take(el_channels.max(0) as usize)
    {
        st.concealment_info.rvlc_previous_block_type =
            if get_window_sequence(&ch.ics_info) == BLOCK_SHORT { 0 } else { 1 };
        if flags & AC_ER_RVLC != 0 {
            st.concealment_info.rvlc_previous_scale_factor_ok =
                ch.p_dyn_data.specific_to.aac.rvlc_current_scale_factor_ok;
        } else {
            st.concealment_info.rvlc_previous_scale_factor_ok = 0;
        }
    }
}
