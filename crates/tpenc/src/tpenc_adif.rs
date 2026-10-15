// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libMpegTPEnc/src/tpenc_adif.h, libMpegTPEnc/src/tpenc_adif.cpp.

//! Port of `tpenc_adif.h`: the ADIF header writer.
//!
//! The code is identical in the `aarch64` and `x86` reference builds.

use fdk_aac_rs_fdk::audio::ChannelMode;
use fdk_aac_rs_fdk::bitstream::{fdk_get_valid_bits, fdk_write_bits, FdkBitStream};

use crate::tpenc_asc::{transport_enc_get_pce_bits, transport_enc_write_pce};
use crate::tpenc_lib::{rd_i32, wr_i32};

/// `ADIF_INFO`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AdifInfo {
    pub cm: ChannelMode,
    pub sampling_rate: i32,
    pub bit_rate: i32,
    pub profile: i32,
    pub b_variable_rate: i32,
    pub instance_tag: i32,
    pub header_written: i32,
    pub matrix_mixdown_a: i32,
    pub pseudo_surround_enable: i32,
}

/// Size of `ADIF_INFO` in the reference builds.
pub const ADIF_INFO_SIZE: usize = 36;

impl AdifInfo {
    /// Reads the structure from its C layout (the writer union of the transport encoder).
    pub fn load(b: &[u8]) -> Self {
        AdifInfo {
            cm: rd_i32(b, 0),
            sampling_rate: rd_i32(b, 4),
            bit_rate: rd_i32(b, 8),
            profile: rd_i32(b, 12),
            b_variable_rate: rd_i32(b, 16),
            instance_tag: rd_i32(b, 20),
            header_written: rd_i32(b, 24),
            matrix_mixdown_a: rd_i32(b, 28),
            pseudo_surround_enable: rd_i32(b, 32),
        }
    }

    /// Writes the structure in its C layout.
    pub fn store(&self, b: &mut [u8]) {
        wr_i32(b, 0, self.cm);
        wr_i32(b, 4, self.sampling_rate);
        wr_i32(b, 8, self.bit_rate);
        wr_i32(b, 12, self.profile);
        wr_i32(b, 16, self.b_variable_rate);
        wr_i32(b, 20, self.instance_tag);
        wr_i32(b, 24, self.header_written);
        wr_i32(b, 28, self.matrix_mixdown_a);
        wr_i32(b, 32, self.pseudo_surround_enable);
    }
}

/// `adifId` of `adifWrite_EncodeHeader()` (function-local in C, `const char adifId[5] = "ADIF"`;
/// the four characters that are written), dumped with
///
/// `scripts/dump_table.py libMpegTPEnc/src/tpenc_adif.cpp adifId_fnlocal UCHAR --prelude 'static const char adifId_fnlocal[5] = "ADIF";' --count 4`
pub const ADIF_ID: [u8; 4] = [65, 68, 73, 70];

/// `adifWrite_EncodeHeader(adif, hBitStream, adif_buffer_fullness)`: writes the ADIF header
/// (with its PCE) unless it was written already. Returns 0.
pub fn adif_write_encode_header(
    adif: &mut AdifInfo,
    h_bs: &mut FdkBitStream,
    adif_buffer_fullness: i32,
) -> i32 {
    // ADIF/PCE/ADTS definitions
    const COPY_RIGHT_ID_PRESENT: bool = false;
    const ORIGINAL_COPY: bool = false;
    const HOME: bool = false;
    let err: i32 = 0;

    let total_bit_rate = adif.bit_rate;

    if adif.header_written != 0 {
        return 0;
    }

    // Align inside PCE with respect to the first bit of the header
    let align_anchor = fdk_get_valid_bits(h_bs);

    // Signal variable bitrate if buffer fullnes exceeds 20 bit
    adif.b_variable_rate = (adif_buffer_fullness >= (1 << 20)) as i32;

    fdk_write_bits(h_bs, ADIF_ID[0] as u32, 8);
    fdk_write_bits(h_bs, ADIF_ID[1] as u32, 8);
    fdk_write_bits(h_bs, ADIF_ID[2] as u32, 8);
    fdk_write_bits(h_bs, ADIF_ID[3] as u32, 8);

    fdk_write_bits(h_bs, COPY_RIGHT_ID_PRESENT as u32, 1);

    if COPY_RIGHT_ID_PRESENT {
        for _ in 0..72 {
            fdk_write_bits(h_bs, 0, 1);
        }
    }
    fdk_write_bits(h_bs, ORIGINAL_COPY as u32, 1);
    fdk_write_bits(h_bs, HOME as u32, 1);
    fdk_write_bits(h_bs, (adif.b_variable_rate != 0) as u32, 1);
    fdk_write_bits(h_bs, total_bit_rate as u32, 23);

    // we write only one PCE at the moment
    fdk_write_bits(h_bs, 0, 4);

    if adif.b_variable_rate == 0 {
        fdk_write_bits(h_bs, adif_buffer_fullness as u32, 20);
    }
    // Write PCE
    transport_enc_write_pce(
        h_bs,
        adif.cm,
        adif.sampling_rate,
        adif.instance_tag,
        adif.profile,
        adif.matrix_mixdown_a,
        (adif.pseudo_surround_enable != 0) as i32,
        align_anchor,
    );

    err
}

/// `adifWrite_GetHeaderBits(adif)`: the bit demand of the ADIF header (0 once it is written).
pub fn adif_write_get_header_bits(adif: &AdifInfo) -> i32 {
    // ADIF definitions
    const COPY_RIGHT_ID_PRESENT: bool = false;

    if adif.header_written != 0 {
        return 0;
    }

    let mut bits: i32 = 0;

    bits += 8 * 4; // ADIF ID

    bits += 1; // Copyright present

    if COPY_RIGHT_ID_PRESENT {
        bits += 72; // Copyright ID
    }

    bits += 26;

    bits += 4; // Number of PCE's

    if adif.b_variable_rate == 0 {
        bits += 20;
    }

    // write PCE
    transport_enc_get_pce_bits(adif.cm, adif.matrix_mixdown_a, bits)
}
