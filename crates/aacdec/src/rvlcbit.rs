// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACdec/src/rvlcbit.h, libAACdec/src/rvlcbit.cpp.

//! Port of `rvlcbit.h`: RVLC decoder, bitstream reading.

use fdk_aac_rs_fdk::bitstream::{
    fdk_get_valid_bits, fdk_push_back, fdk_push_bi_directional, fdk_read_bits, FdkBitStream,
};

use crate::rvlc_info::FWD;

/// `rvlcReadBitFromBitstream(bs, bsAnchor, pPosition, readDirection)`: returns a bit from the
/// bitstream according to the read direction.
pub fn rvlc_read_bit_from_bitstream(
    bs: &mut FdkBitStream,
    bs_anchor: i32,
    p_position: &mut i32,
    read_direction: u8,
) -> u8 {
    let bit;
    // (INT)FDKgetValidBits(bs) - bsAnchor + *pPosition (wraps like the reference; the values stay
    // far from the limits in practice)
    let read_bit_offset: i32 =
        (fdk_get_valid_bits(bs) as i32).wrapping_sub(bs_anchor).wrapping_add(*p_position);

    if read_bit_offset != 0 {
        fdk_push_bi_directional(bs, read_bit_offset);
    }

    if read_direction == FWD {
        bit = fdk_read_bits(bs, 1);

        *p_position = p_position.wrapping_add(1);
    } else {
        /* to be replaced with a brother function of FDKreadBits() */
        bit = fdk_read_bits(bs, 1);
        fdk_push_back(bs, 2);

        *p_position = p_position.wrapping_sub(1);
    }

    bit as u8
}
