// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libSBRenc/src/env_bit.h, libSBRenc/src/env_bit.cpp.

//! Port of `env_bit.h` and `env_bit.cpp`: initialization of the SBR bitstream, CRC and fill
//! bits.

use fdk_aac_rs_fdk::bitbuffer::BitMem;
use fdk_aac_rs_fdk::bitstream::*;
use fdk_aac_rs_fdk::crc::*;
use fdk_aac_rs_fdk::fdk_assert;

use crate::cmondata::CommonData;
use crate::sbr_encoder::{SBR_SYNTAX_CRC, SBR_SYNTAX_DRM_CRC, SBR_SYNTAX_LOW_DELAY};

/// `SBR_CRC_POLY`: G(x) = x^10 + x^9 + x^5 + x^4 + x + 1
pub const SBR_CRC_POLY: u16 = 0x0233;
/// `SBR_CRC_MASK`
pub const SBR_CRC_MASK: u16 = 0x0200;
/// `SBR_CRC_RANGE`
pub const SBR_CRC_RANGE: u16 = 0x03FF;
/// `SBR_CRC_MAXREGS`
pub const SBR_CRC_MAXREGS: i32 = 1;
/// `SBR_CRCINIT`
pub const SBR_CRCINIT: u16 = 0x0;

/// `SI_SBR_CRC_ENABLE_BITS`
pub const SI_SBR_CRC_ENABLE_BITS: u32 = 0;
/// `SI_SBR_CRC_BITS`
pub const SI_SBR_CRC_BITS: u32 = 10;
/// `SI_SBR_DRM_CRC_BITS`
pub const SI_SBR_DRM_CRC_BITS: u32 = 8;

/// `crcAdvance(crcPoly, crcMask, crc, bValue, bBits)`: updates the crc data register.
fn crc_advance(crc_poly: u16, crc_mask: u16, crc: &mut u16, b_value: u32, b_bits: i32) {
    let mut i = b_bits - 1;
    while i >= 0 {
        let mut flag: u16 = if *crc & crc_mask != 0 { 1 } else { 0 };
        flag ^= if b_value & (1u32 << i) != 0 { 1 } else { 0 };

        *crc <<= 1;
        if flag != 0 {
            *crc ^= crc_poly;
        }
        i -= 1;
    }
}

/// `FDKsbrEnc_InitSbrBitstream(hCmonData, memoryBase, memorySize, hCrcInfo,
/// sbrSyntaxFlags)`: initialization of the sbr bitstream, write of the dummy CRC. Returns the
/// CRC region. `memoryBase` is byte `memory_base` of `mem`.
pub fn fdk_sbr_enc_init_sbr_bitstream(
    h_cmon_data: &mut CommonData,
    mem: &BitMem,
    memory_base: u32,
    memory_size: i32,
    h_crc_info: &mut FdkCrcInfo,
    sbr_syntax_flags: u32,
) -> i32 {
    let mut crc_region = 0;

    // reset bit buffer
    fdk_reset_bitbuffer(&mut h_cmon_data.sbr_bitbuf, BS_WRITER);

    fdk_init_bit_stream_at(
        &mut h_cmon_data.tmp_write_bitbuf,
        mem,
        memory_base,
        memory_size as u32,
        0,
        BS_WRITER,
    );

    if sbr_syntax_flags & SBR_SYNTAX_CRC != 0 {
        if sbr_syntax_flags & SBR_SYNTAX_DRM_CRC != 0 {
            // Init and start CRC region
            fdk_write_bits(&mut h_cmon_data.sbr_bitbuf, 0x0, SI_SBR_DRM_CRC_BITS);
            fdk_crc_init(h_crc_info, 0x001d, 0xFFFF, SI_SBR_DRM_CRC_BITS);
            crc_region = fdk_crc_start_reg(h_crc_info, &mut h_cmon_data.sbr_bitbuf, 0);
        } else {
            fdk_write_bits(&mut h_cmon_data.sbr_bitbuf, 0x0, SI_SBR_CRC_BITS);
        }
    }

    crc_region
}

/// `FDKsbrEnc_AssembleSbrBitstream(hCmonData, hCrcInfo, crcRegion, sbrSyntaxFlags)`: formats
/// the SBR payload (fill bits and CRC).
pub fn fdk_sbr_enc_assemble_sbr_bitstream(
    h_cmon_data: Option<&mut CommonData>,
    h_crc_info: &mut FdkCrcInfo,
    crc_region: i32,
    sbr_syntax_flags: u32,
) {
    let mut crc_reg: u16 = SBR_CRCINIT;

    // check if SBR is present
    let Some(h) = h_cmon_data else {
        return;
    };

    h.sbr_fill_bits = 0; // Fill bits are written only for GA streams

    if sbr_syntax_flags & SBR_SYNTAX_DRM_CRC != 0 {
        // Calculate and write DRM CRC
        fdk_crc_end_reg(h_crc_info, &mut h.sbr_bitbuf, crc_region);
        fdk_write_bits(
            &mut h.tmp_write_bitbuf,
            (fdk_crc_get_crc(h_crc_info) ^ 0xFF) as u32,
            SI_SBR_DRM_CRC_BITS,
        );
    } else {
        if sbr_syntax_flags & SBR_SYNTAX_LOW_DELAY == 0 {
            // Do alignment here, because its defined as part of the sbr_extension_data
            let mut sbr_load = h.sbr_hdr_bits + h.sbr_data_bits;

            if sbr_syntax_flags & SBR_SYNTAX_CRC != 0 {
                sbr_load += SI_SBR_CRC_BITS as i32;
            }

            sbr_load += 4; // Do byte Align with 4 bit offset. ISO/IEC 14496-3:2005(E) page 39.

            h.sbr_fill_bits = (8 - (sbr_load % 8)) % 8;

            // append fill bits
            fdk_write_bits(&mut h.sbr_bitbuf, 0, h.sbr_fill_bits as u32);

            // C: FDK_ASSERT(FDKgetValidBits(&hCmonData->sbrBitbuf) % 8 == 4). FDKgetValidBits()
            // flushes the cache, which would change what the CRC below reads (the reference is
            // built with NDEBUG, so the call is never made); the port computes the count
            // without the flush.
            fdk_assert!(
                h.sbr_bitbuf.h_bit_buf.valid_bits.wrapping_add(h.sbr_bitbuf.bits_in_cache) % 8 == 4
            );
        }

        // calculate crc
        if sbr_syntax_flags & SBR_SYNTAX_CRC != 0 {
            let mut tmp_crc_buf = h.sbr_bitbuf.clone();
            fdk_reset_bitbuffer(&mut tmp_crc_buf, BS_READER);

            let num_crc_bits = h.sbr_hdr_bits + h.sbr_data_bits + h.sbr_fill_bits;

            for _ in 0..num_crc_bits.max(0) {
                let bit = fdk_read_bits(&mut tmp_crc_buf, 1);
                crc_advance(SBR_CRC_POLY, SBR_CRC_MASK, &mut crc_reg, bit, 1);
            }
            crc_reg &= SBR_CRC_RANGE;

            // Write CRC data.
            fdk_write_bits(&mut h.tmp_write_bitbuf, crc_reg as u32, SI_SBR_CRC_BITS);
        }
    }

    fdk_sync_cache(&mut h.tmp_write_bitbuf);
}
