// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACdec/src/aacdec_hcr_bit.h, libAACdec/src/aacdec_hcr_bit.cpp.

//! Port of `aacdec_hcr_bit.h`: HCR decoder, bitstream reading.

use fdk_aac_rs_fdk::bitstream::{
    fdk_get_valid_bits, fdk_push_back, fdk_push_bi_directional, fdk_read_bits, FdkBitStream,
};

use crate::aacdec_hcr_types::{FROM_LEFT_TO_RIGHT, FROM_RIGHT_TO_LEFT};

/// `ToggleReadDirection(readDirection)`: toggles the read direction.
pub fn toggle_read_direction(read_direction: u8) -> u8 {
    if read_direction as i32 == FROM_LEFT_TO_RIGHT {
        FROM_RIGHT_TO_LEFT as u8
    } else {
        FROM_LEFT_TO_RIGHT as u8
    }
}

/// `HcrGetABitFromBitstream(bs, bsAnchor, pLeftStartOfSegment, pRightStartOfSegment,
/// readDirection)`: returns a bit from the bitstream according to the read direction.
///
/// The C callers that decode priority codewords pass the same pointer for both segment starts
/// (the right one is a dummy that is only used for `FROM_RIGHT_TO_LEFT`); they use
/// [`hcr_get_a_bit_from_bitstream_ltr`].
pub fn hcr_get_a_bit_from_bitstream(
    bs: &mut FdkBitStream,
    bs_anchor: i32,
    p_left_start_of_segment: &mut i32,
    p_right_start_of_segment: &mut i32,
    read_direction: u8,
) -> u32 {
    let bit;
    let read_bit_offset: i32;

    if read_direction as i32 == FROM_LEFT_TO_RIGHT {
        // (INT)FDKgetValidBits(bs) - bsAnchor + *pLeftStartOfSegment (wraps like the reference;
        // the values stay far from the limits in practice)
        read_bit_offset = (fdk_get_valid_bits(bs) as i32)
            .wrapping_sub(bs_anchor)
            .wrapping_add(*p_left_start_of_segment);
        if read_bit_offset != 0 {
            fdk_push_bi_directional(bs, read_bit_offset);
        }

        bit = fdk_read_bits(bs, 1);

        *p_left_start_of_segment = p_left_start_of_segment.wrapping_add(1);
    } else {
        read_bit_offset = (fdk_get_valid_bits(bs) as i32)
            .wrapping_sub(bs_anchor)
            .wrapping_add(*p_right_start_of_segment);
        if read_bit_offset != 0 {
            fdk_push_bi_directional(bs, read_bit_offset);
        }

        /* to be replaced with a brother function of FDKreadBits() */
        bit = fdk_read_bits(bs, 1);
        fdk_push_back(bs, 2);

        *p_right_start_of_segment = p_right_start_of_segment.wrapping_sub(1);
    }

    bit
}

/// `HcrGetABitFromBitstream(bs, bsAnchor, p, p, FROM_LEFT_TO_RIGHT)`: the calls with the same
/// pointer for both segment starts (`pLeftStartOfSegment, pLeftStartOfSegment /* dummy */`).
#[inline]
pub fn hcr_get_a_bit_from_bitstream_ltr(
    bs: &mut FdkBitStream,
    bs_anchor: i32,
    p_left_start_of_segment: &mut i32,
) -> u32 {
    let mut dummy = *p_left_start_of_segment;
    hcr_get_a_bit_from_bitstream(
        bs,
        bs_anchor,
        p_left_start_of_segment,
        &mut dummy,
        FROM_LEFT_TO_RIGHT as u8,
    )
}
