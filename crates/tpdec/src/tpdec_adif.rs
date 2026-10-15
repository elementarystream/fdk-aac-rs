// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libMpegTPDec/src/tpdec_adif.h, libMpegTPDec/src/tpdec_adif.cpp.

//! Port of `tpdec_adif.h`: the ADIF header parser.
//!
//! The code of this file is identical in the `aarch64` and `x86` reference builds.

use fdk_aac_rs_fdk::bitstream::{
    fdk_byte_align, fdk_get_valid_bits, fdk_push_bi_directional, fdk_read_bits, FdkBitStream,
};

use crate::tp_data::CProgramConfig;
use crate::tpdec_asc::cprogram_config_read;
use crate::tpdec_lib::*;

/// `MIN_ADIF_HEADERLENGTH`: in bits.
pub const MIN_ADIF_HEADERLENGTH: i32 = 63;

/// `CAdifHeader`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CAdifHeader {
    pub num_program_config_elements: i32,
    pub bit_rate: u32,
    pub copyright_id_present: u8,
    pub original_copy: u8,
    pub home: u8,
    pub bitstream_type: u8,
}

/// `adifRead_DecodeHeader(pAdifHeader, pPce, bs)`: parses an ADIF header; all PCEs are read
/// into `p_pce`, the last one remains.
pub fn adif_read_decode_header(
    p_adif_header: &mut CAdifHeader,
    p_pce: &mut CProgramConfig,
    bs: &mut FdkBitStream,
) -> TransportDecError {
    let error_status = TRANSPORTDEC_OK;
    let start_anchor = fdk_get_valid_bits(bs);

    if (start_anchor as i32) < MIN_ADIF_HEADERLENGTH {
        return TRANSPORTDEC_NOT_ENOUGH_BITS;
    }

    if fdk_read_bits(bs, 8) != b'A' as u32 {
        return TRANSPORTDEC_SYNC_ERROR;
    }
    if fdk_read_bits(bs, 8) != b'D' as u32 {
        return TRANSPORTDEC_SYNC_ERROR;
    }
    if fdk_read_bits(bs, 8) != b'I' as u32 {
        return TRANSPORTDEC_SYNC_ERROR;
    }
    if fdk_read_bits(bs, 8) != b'F' as u32 {
        return TRANSPORTDEC_SYNC_ERROR;
    }

    p_adif_header.copyright_id_present = fdk_read_bits(bs, 1) as u8;
    if p_adif_header.copyright_id_present != 0 {
        fdk_push_bi_directional(bs, 72); // CopyrightId
    }

    p_adif_header.original_copy = fdk_read_bits(bs, 1) as u8;
    p_adif_header.home = fdk_read_bits(bs, 1) as u8;
    p_adif_header.bitstream_type = fdk_read_bits(bs, 1) as u8;

    // pAdifHeader->BitRate = FDKreadBits(bs, 23);
    p_adif_header.bit_rate = fdk_read_bits(bs, 16);
    p_adif_header.bit_rate <<= 7;
    p_adif_header.bit_rate |= fdk_read_bits(bs, 7);

    p_adif_header.num_program_config_elements = fdk_read_bits(bs, 4) as i32 + 1;

    if p_adif_header.bitstream_type == 0 {
        fdk_push_bi_directional(bs, 20); // adif_buffer_fullness
    }

    // Parse all PCEs but keep only one
    for _ in 0..p_adif_header.num_program_config_elements {
        cprogram_config_read(p_pce, bs, start_anchor);
    }

    fdk_byte_align(bs, start_anchor);

    error_status
}
