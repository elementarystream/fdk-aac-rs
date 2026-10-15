// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libMpegTPEnc/src/tpenc_adts.h, libMpegTPEnc/src/tpenc_adts.cpp.

//! Port of `tpenc_adts.h`: the ADTS header writer, including the CRC of the header and of the
//! raw data blocks.
//!
//! The code is identical in the `aarch64` and `x86` reference builds (the preprocessed
//! translation units differ only in `FDK_ASSERT`).

use fdk_aac_rs_fdk::audio::*;
use fdk_aac_rs_fdk::bitstream::{
    fdk_get_valid_bits, fdk_init_bit_stream_at, fdk_push_for, fdk_reset_bitbuffer, fdk_sync_cache,
    fdk_write_bits, FdkBitStream, BS_WRITER,
};
use fdk_aac_rs_fdk::crc::{
    fdk_crc_end_reg, fdk_crc_get_crc, fdk_crc_init, fdk_crc_reset, fdk_crc_start_reg, CCrcRegData,
    FdkCrcInfo, CRC_LOOKUP_16_12_5_0, CRC_LOOKUP_16_15_2_0, CRC_LOOKUP_16_15_5_0, MAX_CRC_REGS,
};
use fdk_aac_rs_fdk::fdk_assert;
use fdk_aac_rs_tpdec::tp_data::get_sampling_rate_index;

use crate::tpenc_asc::get_channel_config;
use crate::tpenc_lib::{
    rd_i32, rd_ptr, rd_u16, rd_u32, rd_u8, wr_i32, wr_ptr, wr_u16, wr_u32, wr_u8, ILP32, PTR_SIZE,
};

/// `STRUCT_ADTS`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StructAdts {
    pub sample_freq: i32,
    pub channel_mode: ChannelMode,
    pub decoder_can_do_mpeg4: u8,
    pub mpeg_id: u8,
    pub layer: u8,
    pub protection_absent: u8,
    pub profile: u8,
    pub sample_freq_index: u8,
    pub private_bit: u8,
    pub original: u8,
    pub home: u8,
    pub copyright_id: u8,
    pub copyright_start: u8,
    pub frame_length: u16,
    pub num_raw_blocks: u8,
    pub buffer_fulnes_start_flag: u8,
    pub channel_config_zero: u8,
    /// Header bit demand for the current raw data block
    pub header_bits: i32,
    /// Index of current raw data block
    pub current_block: i32,
    /// Bit position where the current raw data block begins
    pub sub_frame_start_bit: i32,
    pub crc_info: FdkCrcInfo,
}

/// `HANDLE_ADTS`
pub type HandleAdts<'a> = &'a mut StructAdts;

/// Size of `STRUCT_ADTS` in the reference builds (120 bytes on the LP64 builds, 112 with the
/// 4-byte `pCrcLookup` of ref-i686).
pub const STRUCT_ADTS_SIZE: usize = if ILP32 { 112 } else { 120 };

/// Offset of `crcInfo` in `STRUCT_ADTS`.
const OFF_CRC: usize = 40;

/// `pCrcLookup` as stored in a byte image: the C pointer identifies one of the three tables;
/// the port stores the index of the table (0: `NULL`).
fn crc_lookup_id(l: Option<&'static [u16; 256]>) -> u64 {
    match l {
        None => 0,
        Some(t) if core::ptr::eq(t, &CRC_LOOKUP_16_15_5_0) => 1,
        Some(t) if core::ptr::eq(t, &CRC_LOOKUP_16_15_2_0) => 2,
        Some(t) if core::ptr::eq(t, &CRC_LOOKUP_16_12_5_0) => 3,
        Some(_) => 4,
    }
}

fn crc_lookup_from_id(id: u64) -> Option<&'static [u16; 256]> {
    match id {
        1 => Some(&CRC_LOOKUP_16_15_5_0),
        2 => Some(&CRC_LOOKUP_16_15_2_0),
        3 => Some(&CRC_LOOKUP_16_12_5_0),
        _ => None,
    }
}

/// Offset of `crcPoly` in `FDK_CRCINFO`: after the pointer `pCrcLookup` at 48 (56 on the LP64
/// builds, 52 on ref-i686; the fields after it follow without further padding).
const OFF_CRC_POLY: usize = 48 + PTR_SIZE;

/// Reads an `FDK_CRCINFO` from its C layout (80 bytes; 72 on ref-i686) at `o`.
pub fn crc_info_load(b: &[u8], o: usize) -> FdkCrcInfo {
    let mut c = FdkCrcInfo::default();
    for r in 0..MAX_CRC_REGS {
        let ro = o + 16 * r;
        c.crc_reg_data[r] = CCrcRegData {
            is_active: rd_u8(b, ro),
            max_bits: rd_i32(b, ro + 4),
            bit_buf_cnt_bits: rd_i32(b, ro + 8),
            valid_bits: rd_i32(b, ro + 12),
        };
    }
    c.p_crc_lookup = crc_lookup_from_id(rd_ptr(b, o + 48));
    let p = o + OFF_CRC_POLY;
    c.crc_poly = rd_u16(b, p);
    c.crc_mask = rd_u16(b, p + 2);
    c.start_value = rd_u16(b, p + 4);
    c.crc_len = rd_u8(b, p + 6);
    c.reg_start = rd_u32(b, p + 8);
    c.reg_stop = rd_u32(b, p + 12);
    c.crc_value = rd_u16(b, p + 16);
    c
}

/// Writes an `FDK_CRCINFO` in its C layout at `o` (the padding bytes are left unchanged).
pub fn crc_info_store(c: &FdkCrcInfo, b: &mut [u8], o: usize) {
    for r in 0..MAX_CRC_REGS {
        let ro = o + 16 * r;
        let d = &c.crc_reg_data[r];
        wr_u8(b, ro, d.is_active);
        wr_i32(b, ro + 4, d.max_bits);
        wr_i32(b, ro + 8, d.bit_buf_cnt_bits);
        wr_i32(b, ro + 12, d.valid_bits);
    }
    // Leave the bytes untouched if they already identify the same table.
    if crc_lookup_from_id(rd_ptr(b, o + 48)).map(|t| t as *const _)
        != c.p_crc_lookup.map(|t| t as *const _)
    {
        wr_ptr(b, o + 48, crc_lookup_id(c.p_crc_lookup));
    }
    let p = o + OFF_CRC_POLY;
    wr_u16(b, p, c.crc_poly);
    wr_u16(b, p + 2, c.crc_mask);
    wr_u16(b, p + 4, c.start_value);
    wr_u8(b, p + 6, c.crc_len);
    wr_u32(b, p + 8, c.reg_start);
    wr_u32(b, p + 12, c.reg_stop);
    wr_u16(b, p + 16, c.crc_value);
}

impl StructAdts {
    /// Reads the structure from its C layout (the writer union of the transport encoder).
    pub fn load(b: &[u8]) -> Self {
        StructAdts {
            sample_freq: rd_i32(b, 0),
            channel_mode: rd_i32(b, 4),
            decoder_can_do_mpeg4: rd_u8(b, 8),
            mpeg_id: rd_u8(b, 9),
            layer: rd_u8(b, 10),
            protection_absent: rd_u8(b, 11),
            profile: rd_u8(b, 12),
            sample_freq_index: rd_u8(b, 13),
            private_bit: rd_u8(b, 14),
            original: rd_u8(b, 15),
            home: rd_u8(b, 16),
            copyright_id: rd_u8(b, 17),
            copyright_start: rd_u8(b, 18),
            frame_length: rd_u16(b, 20),
            num_raw_blocks: rd_u8(b, 22),
            buffer_fulnes_start_flag: rd_u8(b, 23),
            channel_config_zero: rd_u8(b, 24),
            header_bits: rd_i32(b, 28),
            current_block: rd_i32(b, 32),
            sub_frame_start_bit: rd_i32(b, 36),
            crc_info: crc_info_load(b, OFF_CRC),
        }
    }

    /// Writes the structure in its C layout (the padding bytes are left unchanged).
    pub fn store(&self, b: &mut [u8]) {
        wr_i32(b, 0, self.sample_freq);
        wr_i32(b, 4, self.channel_mode);
        wr_u8(b, 8, self.decoder_can_do_mpeg4);
        wr_u8(b, 9, self.mpeg_id);
        wr_u8(b, 10, self.layer);
        wr_u8(b, 11, self.protection_absent);
        wr_u8(b, 12, self.profile);
        wr_u8(b, 13, self.sample_freq_index);
        wr_u8(b, 14, self.private_bit);
        wr_u8(b, 15, self.original);
        wr_u8(b, 16, self.home);
        wr_u8(b, 17, self.copyright_id);
        wr_u8(b, 18, self.copyright_start);
        wr_u16(b, 20, self.frame_length);
        wr_u8(b, 22, self.num_raw_blocks);
        wr_u8(b, 23, self.buffer_fulnes_start_flag);
        wr_u8(b, 24, self.channel_config_zero);
        wr_i32(b, 28, self.header_bits);
        wr_i32(b, 32, self.current_block);
        wr_i32(b, 36, self.sub_frame_start_bit);
        crc_info_store(&self.crc_info, b, OFF_CRC);
    }
}

/// `adtsWrite_CrcStartReg(pAdts, hBs, mBits)`: starts a CRC region (`mBits` as for
/// `FDKcrcStartReg()`). Returns the region ID, 0 without protection.
pub fn adts_write_crc_start_reg(
    p_adts: &mut StructAdts,
    h_bs: &mut FdkBitStream,
    m_bits: i32,
) -> i32 {
    if p_adts.protection_absent != 0 {
        return 0;
    }
    fdk_crc_start_reg(&mut p_adts.crc_info, h_bs, m_bits)
}

/// `adtsWrite_CrcEndReg(pAdts, hBs, reg)`: ends the CRC region `reg`.
pub fn adts_write_crc_end_reg(p_adts: &mut StructAdts, h_bs: &mut FdkBitStream, reg: i32) {
    if p_adts.protection_absent == 0 {
        fdk_crc_end_reg(&mut p_adts.crc_info, h_bs, reg);
    }
}

/// `adtsWrite_GetHeaderBits(hAdts)`: the bit overhead of ADTS for the current raw data block
/// (also stored in `header_bits`).
pub fn adts_write_get_header_bits(h_adts: &mut StructAdts) -> i32 {
    let mut bits: i32 = 0;

    if h_adts.current_block == 0 {
        // Static and variable header bits
        bits = 56;
        if h_adts.protection_absent == 0 {
            // Add header/ single raw data block CRC bits
            bits += 16;
            if h_adts.num_raw_blocks > 0 {
                // Add bits of raw data block position markers
                bits += h_adts.num_raw_blocks as i32 * 16;
            }
        }
    }
    if h_adts.protection_absent == 0 && h_adts.num_raw_blocks > 0 {
        // Add raw data block CRC bits. Not really part of the header, put they cause bit
        // overhead to be accounted.
        bits += 16;
    }

    h_adts.header_bits = bits;

    bits
}

/// `adtsWrite_Init(hAdts, config)`: initializes the ADTS writer. Returns 0 on success, -1 for
/// a configuration ADTS cannot carry.
pub fn adts_write_init(h_adts: &mut StructAdts, config: &CoderConfig) -> i32 {
    // Sanity checks
    if config.n_sub_frames < 1 || config.n_sub_frames > 4 || config.aot > 4 || config.aot < 1 {
        return -1;
    }

    // fixed header
    if (config.flags & CC_MPEG_ID) != 0 {
        h_adts.mpeg_id = 0; // MPEG 4
    } else {
        h_adts.mpeg_id = 1; // MPEG 2
    }
    h_adts.layer = 0;
    h_adts.protection_absent = ((config.flags & CC_PROTECTION) == 0) as u8;
    h_adts.profile = (config.aot - 1) as u8;
    h_adts.sample_freq_index = get_sampling_rate_index(config.sampling_rate as u32, 4) as u8;
    h_adts.sample_freq = config.sampling_rate;
    h_adts.private_bit = 0;
    h_adts.channel_mode = config.channel_mode;
    h_adts.original = 0;
    h_adts.home = 0;
    // variable header
    h_adts.copyright_id = 0;
    h_adts.copyright_start = 0;

    h_adts.num_raw_blocks = (config.n_sub_frames - 1) as u8; // 0 means 1 raw data block

    h_adts.channel_config_zero = config.channel_config_zero;

    fdk_crc_init(&mut h_adts.crc_info, 0x8005, 0xFFFF, 16);

    h_adts.current_block = 0;

    0
}

/// `adtsWrite_EncodeHeader(hAdts, hBitStream, buffer_fullness, frame_length)`: writes the ADTS
/// header for the first raw data block of a frame (nothing for the further blocks). Returns 0.
pub fn adts_write_encode_header(
    h_adts: &mut StructAdts,
    h_bit_stream: &mut FdkBitStream,
    buffer_fullness: i32,
    frame_length: i32,
) -> i32 {
    let mut crc_index: i32 = 0;

    h_adts.header_bits = adts_write_get_header_bits(h_adts);

    fdk_assert!((frame_length as i64 + h_adts.header_bits as i64) / 8 < 0x2000); // 13 bit
    fdk_assert!(buffer_fullness < 0x800); // 11 bit

    if h_adts.protection_absent == 0 {
        fdk_crc_reset(&mut h_adts.crc_info);
    }

    if h_adts.current_block == 0 {
        fdk_reset_bitbuffer(h_bit_stream, BS_WRITER);
    }

    h_adts.sub_frame_start_bit = fdk_get_valid_bits(h_bit_stream) as i32;

    // Skip new header if this is raw data block 1..n
    if h_adts.current_block == 0 {
        fdk_reset_bitbuffer(h_bit_stream, BS_WRITER);

        if h_adts.num_raw_blocks == 0 {
            crc_index = adts_write_crc_start_reg(h_adts, h_bit_stream, 0);
        }

        // fixed header
        fdk_write_bits(h_bit_stream, 0xFFF, 12);
        fdk_write_bits(h_bit_stream, h_adts.mpeg_id as u32, 1);
        fdk_write_bits(h_bit_stream, h_adts.layer as u32, 2);
        fdk_write_bits(h_bit_stream, h_adts.protection_absent as u32, 1);
        fdk_write_bits(h_bit_stream, h_adts.profile as u32, 2);
        fdk_write_bits(h_bit_stream, h_adts.sample_freq_index as u32, 4);
        fdk_write_bits(h_bit_stream, h_adts.private_bit as u32, 1);
        fdk_write_bits(
            h_bit_stream,
            get_channel_config(h_adts.channel_mode, h_adts.channel_config_zero) as u32,
            3,
        );
        fdk_write_bits(h_bit_stream, h_adts.original as u32, 1);
        fdk_write_bits(h_bit_stream, h_adts.home as u32, 1);
        // variable header
        fdk_write_bits(h_bit_stream, h_adts.copyright_id as u32, 1);
        fdk_write_bits(h_bit_stream, h_adts.copyright_start as u32, 1);
        fdk_write_bits(
            h_bit_stream,
            (frame_length.wrapping_add(h_adts.header_bits) >> 3) as u32,
            13,
        );
        fdk_write_bits(h_bit_stream, buffer_fullness as u32, 11);
        fdk_write_bits(h_bit_stream, h_adts.num_raw_blocks as u32, 2);

        if h_adts.protection_absent == 0 {
            // End header CRC portion for single raw data block and write dummy zero values for
            // unknown fields.
            if h_adts.num_raw_blocks == 0 {
                adts_write_crc_end_reg(h_adts, h_bit_stream, crc_index);
            } else {
                for _ in 0..h_adts.num_raw_blocks {
                    fdk_write_bits(h_bit_stream, 0, 16);
                }
            }
            fdk_write_bits(h_bit_stream, 0, 16);
        }
    } // End of ADTS header

    0
}

/// `adtsWrite_EndRawDataBlock(hAdts, hBs, pBits)`: finishes a raw data block (CRC, block
/// position markers, total frame length and header CRC after the last block) and corrects
/// `*p_bits` to the bits of the current raw data block.
pub fn adts_write_end_raw_data_block(
    h_adts: &mut StructAdts,
    h_bs: &mut FdkBitStream,
    p_bits: &mut i32,
) {
    if h_adts.protection_absent == 0 {
        let mut bs_writer = FdkBitStream::default();

        fdk_init_bit_stream_at(
            &mut bs_writer,
            h_bs.h_bit_buf.mem(),
            h_bs.h_bit_buf.base(),
            h_bs.h_bit_buf.buf_size(),
            0,
            BS_WRITER,
        );
        fdk_push_for(&mut bs_writer, 56);

        if h_adts.num_raw_blocks == 0 {
            fdk_write_bits(&mut bs_writer, fdk_crc_get_crc(&h_adts.crc_info) as u32, 16);
        } else {
            // Write CRC of current raw data block
            fdk_write_bits(h_bs, fdk_crc_get_crc(&h_adts.crc_info) as u32, 16);

            // Write distance to current data block
            if h_adts.current_block < h_adts.num_raw_blocks as i32 {
                fdk_push_for(&mut bs_writer, h_adts.current_block.wrapping_mul(16) as u32);
                // UINT - int, converted to int
                let distance = fdk_get_valid_bits(h_bs)
                    .wrapping_sub((56 + h_adts.num_raw_blocks as i32 * 16 + 16) as u32)
                    as i32;
                fdk_write_bits(&mut bs_writer, (distance >> 3) as u32, 16);
            }
        }
        fdk_sync_cache(&mut bs_writer);
    }

    // Write total frame lenth for multiple raw data blocks and header CRC
    if h_adts.num_raw_blocks > 0 && h_adts.current_block == h_adts.num_raw_blocks as i32 {
        let mut bs_writer = FdkBitStream::default();
        let mut crc_index: i32 = 0;

        fdk_init_bit_stream_at(
            &mut bs_writer,
            h_bs.h_bit_buf.mem(),
            h_bs.h_bit_buf.base(),
            h_bs.h_bit_buf.buf_size(),
            0,
            BS_WRITER,
        );

        if h_adts.protection_absent == 0 {
            fdk_crc_reset(&mut h_adts.crc_info);
            crc_index = fdk_crc_start_reg(&mut h_adts.crc_info, &mut bs_writer, 0);
        }
        // Write total frame length
        fdk_push_for(&mut bs_writer, 56 - 28 + 2);
        fdk_write_bits(&mut bs_writer, fdk_get_valid_bits(h_bs) >> 3, 13);

        // Write header CRC
        if h_adts.protection_absent == 0 {
            fdk_push_for(&mut bs_writer, (11 + 2 + h_adts.num_raw_blocks as i32 * 16) as u32);
            fdk_crc_end_reg(&mut h_adts.crc_info, &mut bs_writer, crc_index);
            fdk_write_bits(&mut bs_writer, fdk_crc_get_crc(&h_adts.crc_info) as u32, 16);
        }
        fdk_sync_cache(&mut bs_writer);
    }

    // Correct *pBits to reflect the amount of bits of the current subframe
    *p_bits = p_bits.wrapping_sub(h_adts.sub_frame_start_bit);
    if h_adts.protection_absent == 0 && h_adts.num_raw_blocks > 0 {
        // Fixup CRC bits, since they come after each raw data block
        *p_bits = p_bits.wrapping_add(16);
    }
    h_adts.current_block = h_adts.current_block.wrapping_add(1);
}
