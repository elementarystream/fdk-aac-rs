// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libSBRdec/src/huff_dec.h, libSBRdec/src/huff_dec.cpp.

//! Port of `huff_dec.h` and `huff_dec.cpp`: the Huffman decoder of the SBR envelope and noise
//! data.

use fdk_aac_rs_fdk::bitstream::{fdk_read_bits, FdkBitStream};

/// `Huffman` (`const SCHAR (*)[2]`): a Huffman codebook. An entry is the index of the next node
/// or, if negative, the code word minus 64.
pub type Huffman = &'static [[i8; 2]];

/// `DecodeHuffmanCW(h, hBs)`: reads bits until a code word of `h` is complete and returns its
/// value.
///
/// The codebooks of the library only contain indices of their own nodes, so the walk stays
/// inside the table.
pub fn decode_huffman_cw(h: Huffman, h_bs: &mut FdkBitStream) -> i32 {
    let mut index: i8 = 0;

    while index >= 0 {
        let bit = fdk_read_bits(h_bs, 1);
        index = h[index as usize][bit as usize];
    }

    // Add offset
    index as i32 + 64
}
