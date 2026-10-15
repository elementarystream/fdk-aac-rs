// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libFDK/include/FDK_crc.h, libFDK/src/FDK_crc.cpp.

//! Port of `FDK_crc.h`: CRC calculation over bitstream regions.
//!
//! `FDK_crc.cpp` is identical in the `aarch64` and `x86` reference builds (the preprocessed
//! translation units differ only in `FDK_ASSERT`).

use crate::bitstream::{
    fdk_get_valid_bits, fdk_init_bit_stream_at, fdk_push_bi_directional, fdk_read_bit,
    fdk_read_bits, FdkBitStream, BS_READER, BS_WRITER,
};

/// `MAX_CRC_REGS`: maximal number of overlapping crc regions in ADTS channel pair element is two.
/// Select three independent regions preventively.
pub const MAX_CRC_REGS: usize = 3;

/// `CCrcRegData`: describes a single crc region used for crc calculation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CCrcRegData {
    pub is_active: u8,
    pub max_bits: i32,
    pub bit_buf_cnt_bits: i32,
    pub valid_bits: i32,
}

/// `FDK_CRCINFO`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FdkCrcInfo {
    /// Multiple crc region description.
    pub crc_reg_data: [CCrcRegData; MAX_CRC_REGS],
    /// Lookup table selected in `FDKcrcInit()` (`pCrcLookup`).
    pub p_crc_lookup: Option<&'static [u16; 256]>,
    /// CRC generator polynom.
    pub crc_poly: u16,
    /// CRC mask.
    pub crc_mask: u16,
    /// CRC start value.
    pub start_value: u16,
    /// CRC length.
    pub crc_len: u8,
    /// Start region marker for synchronization.
    pub reg_start: u32,
    /// Stop region marker for synchronization.
    pub reg_stop: u32,
    /// Crc value to be calculated.
    pub crc_value: u16,
}

/// `HANDLE_FDK_CRCINFO`
pub type HandleFdkCrcInfo<'a> = &'a mut FdkCrcInfo;

/// `FDKcrcInit(hCrcInfo, crcPoly, crcStartValue, crcLen)`: initializes the crc structure with
/// the given configuration.
pub fn fdk_crc_init(
    h_crc_info: &mut FdkCrcInfo,
    crc_poly: u32,
    crc_start_value: u32,
    crc_len: u32,
) {
    // crc polynom example:
    // x^16 + x^15 + x^5 + x^0        (1) 1000 0000 0010 0001 -> 0x8021
    // x^16 + x^15 + x^2 + x^0        (1) 1000 0000 0000 0101 -> 0x8005
    // x^16 + x^12 + x^5 + x^0        (1) 0001 0000 0010 0001 -> 0x1021
    // x^8 + x^4 + x^3 + x^2 + x^0              (1) 0001 1101 -> 0x001d

    h_crc_info.crc_len = crc_len as u8;
    h_crc_info.crc_poly = crc_poly as u16;
    h_crc_info.start_value = crc_start_value as u16;
    // `(crcLen) ? (1 << (crcLen - 1)) : 0`, stored as USHORT
    h_crc_info.crc_mask = if crc_len != 0 { 1i32.wrapping_shl(crc_len - 1) as u16 } else { 0 };

    fdk_crc_reset(h_crc_info);

    // Preset 0 for "crcLen" != 16 or unknown 16-bit polynoms "crcPoly"
    h_crc_info.p_crc_lookup = None;

    if h_crc_info.crc_len == 16 {
        match crc_poly {
            0x8021 => h_crc_info.p_crc_lookup = Some(&CRC_LOOKUP_16_15_5_0),
            0x8005 => h_crc_info.p_crc_lookup = Some(&CRC_LOOKUP_16_15_2_0),
            0x1021 => h_crc_info.p_crc_lookup = Some(&CRC_LOOKUP_16_12_5_0),
            // 0x001d and others: no lookup table
            _ => {}
        }
    }
}

/// `FDKcrcReset(hCrcInfo)`: clears all intern states of the crc structure.
pub fn fdk_crc_reset(h_crc_info: &mut FdkCrcInfo) {
    h_crc_info.crc_value = h_crc_info.start_value;

    for r in h_crc_info.crc_reg_data.iter_mut() {
        r.is_active = 0;
    }
    h_crc_info.reg_start = 0;
    h_crc_info.reg_stop = 0;
}

/// `FDKcrcStartReg(hCrcInfo, hBs, mBits)`: marks the current bitstream position as start of a
/// crc region and returns its ID.
///
/// - `m_bits > 0`: zero padding is used if fewer than `m_bits` bits are available,
/// - `m_bits < 0`: no zero padding,
/// - `m_bits == 0`: the region length is the distance between the start and the end position.
pub fn fdk_crc_start_reg(h_crc_info: &mut FdkCrcInfo, h_bs: &mut FdkBitStream, m_bits: i32) -> i32 {
    let reg = h_crc_info.reg_start as usize;

    // FDK_ASSERT(isActive == 0) is compiled out in the reference build.
    let valid_bits = fdk_get_valid_bits(h_bs) as i32;
    let r = &mut h_crc_info.crc_reg_data[reg % MAX_CRC_REGS];
    r.is_active = 1;
    r.max_bits = m_bits;
    r.valid_bits = valid_bits;
    r.bit_buf_cnt_bits = 0;

    h_crc_info.reg_start = (h_crc_info.reg_start + 1) % MAX_CRC_REGS as u32;

    reg as i32
}

/// `FDKcrcEndReg(hCrcInfo, hBs, reg)`: terminates the crc region `reg` and updates the crc.
///
/// `reg` must be a region ID (0..3), as returned by `fdk_crc_start_reg()`. Other values have no
/// defined result in C; the port ignores them (never passed by the callers).
pub fn fdk_crc_end_reg(h_crc_info: &mut FdkCrcInfo, h_bs: &mut FdkBitStream, reg: i32) -> i32 {
    // FDK_ASSERT(reg == regStop && isActive == 1) is compiled out in the reference build.
    fdk_assert!((0..MAX_CRC_REGS as i32).contains(&reg));
    if !(0..MAX_CRC_REGS as i32).contains(&reg) {
        return 0;
    }
    let r = reg as usize;

    if h_bs.config_cache == BS_WRITER {
        h_crc_info.crc_reg_data[r].bit_buf_cnt_bits =
            (fdk_get_valid_bits(h_bs) as i32).wrapping_sub(h_crc_info.crc_reg_data[r].valid_bits);
    } else {
        h_crc_info.crc_reg_data[r].bit_buf_cnt_bits =
            h_crc_info.crc_reg_data[r].valid_bits.wrapping_sub(fdk_get_valid_bits(h_bs) as i32);
    }

    if h_crc_info.crc_reg_data[r].max_bits == 0 {
        h_crc_info.crc_reg_data[r].max_bits = h_crc_info.crc_reg_data[r].bit_buf_cnt_bits;
    }

    crc_calc(h_crc_info, h_bs, r);

    h_crc_info.crc_reg_data[r].is_active = 0;
    h_crc_info.reg_stop = (h_crc_info.reg_stop + 1) % MAX_CRC_REGS as u32;

    0
}

/// `FDKcrcGetCRC(hCrcInfo)`: the crc value masked with the crc length.
pub fn fdk_crc_get_crc(h_crc_info: &FdkCrcInfo) -> u16 {
    // `crcValue & (((crcMask - 1) << 1) + 1)` in int
    ((h_crc_info.crc_value as i32) & ((((h_crc_info.crc_mask as i32) - 1) << 1) + 1)) as u16
}

/// `calcCrc_Bits()`: calculates the crc over `n_bits` bits from the current bitstream position,
/// or over `n_bits` zero bits without a bitstream. Returns the number of processed bits.
#[inline]
fn calc_crc_bits(
    p_crc: &mut u16,
    crc_mask: u16,
    crc_poly: u16,
    h_bs: Option<&mut FdkBitStream>,
    n_bits: i32,
) -> i32 {
    let mut crc = *p_crc; // get crc value

    if let Some(h_bs) = h_bs {
        for _ in 0..n_bits {
            let mut tmp = fdk_read_bit(h_bs) as u16; // process single bit
            tmp ^= if crc & crc_mask != 0 { 1 } else { 0 };
            if tmp != 0 {
                tmp = crc_poly;
            }
            crc <<= 1;
            crc ^= tmp;
        }
    } else {
        for _ in 0..n_bits {
            let tmp = if crc & crc_mask != 0 { crc_poly } else { 0 }; // process single bit
            crc <<= 1;
            crc ^= tmp;
        }
    }
    *p_crc = crc; // update crc value

    n_bits
}

/// `calcCrc_Bytes()`: calculates the crc over `n_bytes` bytes from the current bitstream
/// position, or over `n_bytes` zero bytes without a bitstream. Returns the number of processed
/// bytes.
#[inline]
fn calc_crc_bytes(
    p_crc: &mut u16,
    p_crc_lookup: &[u16; 256],
    h_bs: Option<&mut FdkBitStream>,
    n_bytes: i32,
) -> i32 {
    let mut crc = *p_crc; // get crc value
                          // `(crc << 8) ^ pCrcLookup[((crc >> 8) ^ (USHORT)(data >> s)) & 0xFF]`, stored as USHORT
    let step = |crc: u16, b: u32| -> u16 {
        (crc << 8) ^ p_crc_lookup[(((crc >> 8) ^ (b as u16)) & 0xFF) as usize]
    };

    if let Some(h_bs) = h_bs {
        for _ in 0..(n_bytes >> 2) {
            let data = fdk_read_bits(h_bs, 32);
            crc = step(crc, data >> 24);
            crc = step(crc, data >> 16);
            crc = step(crc, data >> 8);
            crc = step(crc, data);
        }
        let mut bits = (n_bytes & 3) << 3;
        if bits > 0 {
            let data = fdk_read_bits(h_bs, bits as u32);
            bits -= 8;
            while bits >= 0 {
                crc = step(crc, data >> bits);
                bits -= 8;
            }
        }
    } else {
        for _ in 0..n_bytes {
            crc = (crc << 8) ^ p_crc_lookup[((crc >> 8) & 0xFF) as usize];
        }
    }

    *p_crc = crc; // update crc value

    n_bytes
}

/// `crcCalc()`: calculates the crc of region `reg`. The length depends on the `mBits` parameter
/// of `FDKcrcStartReg()`.
fn crc_calc(h_crc_info: &mut FdkCrcInfo, h_bs: &mut FdkBitStream, reg: usize) {
    let mut crc = h_crc_info.crc_value;
    let rd = h_crc_info.crc_reg_data[reg];
    let mut bs_reader: FdkBitStream;

    if h_bs.config_cache == BS_READER {
        bs_reader = h_bs.clone();
        let vb = fdk_get_valid_bits(&mut bs_reader) as i32;
        fdk_push_bi_directional(&mut bs_reader, rd.valid_bits.wrapping_sub(vb).wrapping_neg());
    } else {
        bs_reader = FdkBitStream::default();
        // The reader uses the memory of `h_bs`.
        fdk_init_bit_stream_at(
            &mut bs_reader,
            h_bs.h_bit_buf.mem(),
            h_bs.h_bit_buf.base(),
            h_bs.h_bit_buf.buf_size(),
            h_bs.h_bit_buf.valid_bits,
            BS_READER,
        );
        fdk_push_bi_directional(&mut bs_reader, rd.valid_bits);
    }

    // remaining bits
    let mut r_bits = if rd.max_bits >= 0 { rd.max_bits } else { rd.max_bits.wrapping_neg() };
    let bits = if rd.max_bits > 0 && ((rd.bit_buf_cnt_bits >> 3) << 3) < r_bits {
        rd.bit_buf_cnt_bits
    } else {
        r_bits
    };

    let words = bits >> 3; // processing bytes
    let m_bits = bits & 0x7; // modulo bits

    if let Some(lookup) = h_crc_info.p_crc_lookup {
        r_bits =
            r_bits.wrapping_sub(calc_crc_bytes(&mut crc, lookup, Some(&mut bs_reader), words) << 3);
    } else {
        r_bits = r_bits.wrapping_sub(calc_crc_bits(
            &mut crc,
            h_crc_info.crc_mask,
            h_crc_info.crc_poly,
            Some(&mut bs_reader),
            words << 3,
        ));
    }

    // remaining valid bits
    if m_bits != 0 {
        r_bits = r_bits.wrapping_sub(calc_crc_bits(
            &mut crc,
            h_crc_info.crc_mask,
            h_crc_info.crc_poly,
            Some(&mut bs_reader),
            m_bits,
        ));
    }

    if r_bits != 0 {
        // zero bytes
        if let Some(lookup) = h_crc_info.p_crc_lookup {
            if r_bits > 8 {
                r_bits -= calc_crc_bytes(&mut crc, lookup, None, r_bits >> 3) << 3;
            }
        }
        // remaining zero bits
        if r_bits != 0 {
            calc_crc_bits(&mut crc, h_crc_info.crc_mask, h_crc_info.crc_poly, None, r_bits);
        }
    }

    h_crc_info.crc_value = crc;
}

// The lookup tables are generated with
//   scripts/dump_table.py libFDK/src/FDK_crc.cpp crcLookup_16_15_2_0 USHORT --hex \
//     --name CRC_LOOKUP_16_15_2_0 --per-line 12
// and likewise for crcLookup_16_12_5_0 and crcLookup_16_15_5_0.

/// `crcLookup_16_15_2_0`: lookup table for the crc polynom x^16 + x^15 + x^2 + x^0.
pub static CRC_LOOKUP_16_15_2_0: [u16; 256] = [
    0x0000, 0x8005, 0x800F, 0x000A, 0x801B, 0x001E, 0x0014, 0x8011, 0x8033, 0x0036, 0x003C, 0x8039,
    0x0028, 0x802D, 0x8027, 0x0022, 0x8063, 0x0066, 0x006C, 0x8069, 0x0078, 0x807D, 0x8077, 0x0072,
    0x0050, 0x8055, 0x805F, 0x005A, 0x804B, 0x004E, 0x0044, 0x8041, 0x80C3, 0x00C6, 0x00CC, 0x80C9,
    0x00D8, 0x80DD, 0x80D7, 0x00D2, 0x00F0, 0x80F5, 0x80FF, 0x00FA, 0x80EB, 0x00EE, 0x00E4, 0x80E1,
    0x00A0, 0x80A5, 0x80AF, 0x00AA, 0x80BB, 0x00BE, 0x00B4, 0x80B1, 0x8093, 0x0096, 0x009C, 0x8099,
    0x0088, 0x808D, 0x8087, 0x0082, 0x8183, 0x0186, 0x018C, 0x8189, 0x0198, 0x819D, 0x8197, 0x0192,
    0x01B0, 0x81B5, 0x81BF, 0x01BA, 0x81AB, 0x01AE, 0x01A4, 0x81A1, 0x01E0, 0x81E5, 0x81EF, 0x01EA,
    0x81FB, 0x01FE, 0x01F4, 0x81F1, 0x81D3, 0x01D6, 0x01DC, 0x81D9, 0x01C8, 0x81CD, 0x81C7, 0x01C2,
    0x0140, 0x8145, 0x814F, 0x014A, 0x815B, 0x015E, 0x0154, 0x8151, 0x8173, 0x0176, 0x017C, 0x8179,
    0x0168, 0x816D, 0x8167, 0x0162, 0x8123, 0x0126, 0x012C, 0x8129, 0x0138, 0x813D, 0x8137, 0x0132,
    0x0110, 0x8115, 0x811F, 0x011A, 0x810B, 0x010E, 0x0104, 0x8101, 0x8303, 0x0306, 0x030C, 0x8309,
    0x0318, 0x831D, 0x8317, 0x0312, 0x0330, 0x8335, 0x833F, 0x033A, 0x832B, 0x032E, 0x0324, 0x8321,
    0x0360, 0x8365, 0x836F, 0x036A, 0x837B, 0x037E, 0x0374, 0x8371, 0x8353, 0x0356, 0x035C, 0x8359,
    0x0348, 0x834D, 0x8347, 0x0342, 0x03C0, 0x83C5, 0x83CF, 0x03CA, 0x83DB, 0x03DE, 0x03D4, 0x83D1,
    0x83F3, 0x03F6, 0x03FC, 0x83F9, 0x03E8, 0x83ED, 0x83E7, 0x03E2, 0x83A3, 0x03A6, 0x03AC, 0x83A9,
    0x03B8, 0x83BD, 0x83B7, 0x03B2, 0x0390, 0x8395, 0x839F, 0x039A, 0x838B, 0x038E, 0x0384, 0x8381,
    0x0280, 0x8285, 0x828F, 0x028A, 0x829B, 0x029E, 0x0294, 0x8291, 0x82B3, 0x02B6, 0x02BC, 0x82B9,
    0x02A8, 0x82AD, 0x82A7, 0x02A2, 0x82E3, 0x02E6, 0x02EC, 0x82E9, 0x02F8, 0x82FD, 0x82F7, 0x02F2,
    0x02D0, 0x82D5, 0x82DF, 0x02DA, 0x82CB, 0x02CE, 0x02C4, 0x82C1, 0x8243, 0x0246, 0x024C, 0x8249,
    0x0258, 0x825D, 0x8257, 0x0252, 0x0270, 0x8275, 0x827F, 0x027A, 0x826B, 0x026E, 0x0264, 0x8261,
    0x0220, 0x8225, 0x822F, 0x022A, 0x823B, 0x023E, 0x0234, 0x8231, 0x8213, 0x0216, 0x021C, 0x8219,
    0x0208, 0x820D, 0x8207, 0x0202,
];

/// `crcLookup_16_12_5_0`: lookup table for the crc polynom x^16 + x^12 + x^5 + x^0.
pub static CRC_LOOKUP_16_12_5_0: [u16; 256] = [
    0x0000, 0x1021, 0x2042, 0x3063, 0x4084, 0x50A5, 0x60C6, 0x70E7, 0x8108, 0x9129, 0xA14A, 0xB16B,
    0xC18C, 0xD1AD, 0xE1CE, 0xF1EF, 0x1231, 0x0210, 0x3273, 0x2252, 0x52B5, 0x4294, 0x72F7, 0x62D6,
    0x9339, 0x8318, 0xB37B, 0xA35A, 0xD3BD, 0xC39C, 0xF3FF, 0xE3DE, 0x2462, 0x3443, 0x0420, 0x1401,
    0x64E6, 0x74C7, 0x44A4, 0x5485, 0xA56A, 0xB54B, 0x8528, 0x9509, 0xE5EE, 0xF5CF, 0xC5AC, 0xD58D,
    0x3653, 0x2672, 0x1611, 0x0630, 0x76D7, 0x66F6, 0x5695, 0x46B4, 0xB75B, 0xA77A, 0x9719, 0x8738,
    0xF7DF, 0xE7FE, 0xD79D, 0xC7BC, 0x48C4, 0x58E5, 0x6886, 0x78A7, 0x0840, 0x1861, 0x2802, 0x3823,
    0xC9CC, 0xD9ED, 0xE98E, 0xF9AF, 0x8948, 0x9969, 0xA90A, 0xB92B, 0x5AF5, 0x4AD4, 0x7AB7, 0x6A96,
    0x1A71, 0x0A50, 0x3A33, 0x2A12, 0xDBFD, 0xCBDC, 0xFBBF, 0xEB9E, 0x9B79, 0x8B58, 0xBB3B, 0xAB1A,
    0x6CA6, 0x7C87, 0x4CE4, 0x5CC5, 0x2C22, 0x3C03, 0x0C60, 0x1C41, 0xEDAE, 0xFD8F, 0xCDEC, 0xDDCD,
    0xAD2A, 0xBD0B, 0x8D68, 0x9D49, 0x7E97, 0x6EB6, 0x5ED5, 0x4EF4, 0x3E13, 0x2E32, 0x1E51, 0x0E70,
    0xFF9F, 0xEFBE, 0xDFDD, 0xCFFC, 0xBF1B, 0xAF3A, 0x9F59, 0x8F78, 0x9188, 0x81A9, 0xB1CA, 0xA1EB,
    0xD10C, 0xC12D, 0xF14E, 0xE16F, 0x1080, 0x00A1, 0x30C2, 0x20E3, 0x5004, 0x4025, 0x7046, 0x6067,
    0x83B9, 0x9398, 0xA3FB, 0xB3DA, 0xC33D, 0xD31C, 0xE37F, 0xF35E, 0x02B1, 0x1290, 0x22F3, 0x32D2,
    0x4235, 0x5214, 0x6277, 0x7256, 0xB5EA, 0xA5CB, 0x95A8, 0x8589, 0xF56E, 0xE54F, 0xD52C, 0xC50D,
    0x34E2, 0x24C3, 0x14A0, 0x0481, 0x7466, 0x6447, 0x5424, 0x4405, 0xA7DB, 0xB7FA, 0x8799, 0x97B8,
    0xE75F, 0xF77E, 0xC71D, 0xD73C, 0x26D3, 0x36F2, 0x0691, 0x16B0, 0x6657, 0x7676, 0x4615, 0x5634,
    0xD94C, 0xC96D, 0xF90E, 0xE92F, 0x99C8, 0x89E9, 0xB98A, 0xA9AB, 0x5844, 0x4865, 0x7806, 0x6827,
    0x18C0, 0x08E1, 0x3882, 0x28A3, 0xCB7D, 0xDB5C, 0xEB3F, 0xFB1E, 0x8BF9, 0x9BD8, 0xABBB, 0xBB9A,
    0x4A75, 0x5A54, 0x6A37, 0x7A16, 0x0AF1, 0x1AD0, 0x2AB3, 0x3A92, 0xFD2E, 0xED0F, 0xDD6C, 0xCD4D,
    0xBDAA, 0xAD8B, 0x9DE8, 0x8DC9, 0x7C26, 0x6C07, 0x5C64, 0x4C45, 0x3CA2, 0x2C83, 0x1CE0, 0x0CC1,
    0xEF1F, 0xFF3E, 0xCF5D, 0xDF7C, 0xAF9B, 0xBFBA, 0x8FD9, 0x9FF8, 0x6E17, 0x7E36, 0x4E55, 0x5E74,
    0x2E93, 0x3EB2, 0x0ED1, 0x1EF0,
];

/// `crcLookup_16_15_5_0`: lookup table for the crc polynom x^16 + x^15 + x^5 + x^0.
pub static CRC_LOOKUP_16_15_5_0: [u16; 256] = [
    0x0000, 0x8021, 0x8063, 0x0042, 0x80E7, 0x00C6, 0x0084, 0x80A5, 0x81EF, 0x01CE, 0x018C, 0x81AD,
    0x0108, 0x8129, 0x816B, 0x014A, 0x83FF, 0x03DE, 0x039C, 0x83BD, 0x0318, 0x8339, 0x837B, 0x035A,
    0x0210, 0x8231, 0x8273, 0x0252, 0x82F7, 0x02D6, 0x0294, 0x82B5, 0x87DF, 0x07FE, 0x07BC, 0x879D,
    0x0738, 0x8719, 0x875B, 0x077A, 0x0630, 0x8611, 0x8653, 0x0672, 0x86D7, 0x06F6, 0x06B4, 0x8695,
    0x0420, 0x8401, 0x8443, 0x0462, 0x84C7, 0x04E6, 0x04A4, 0x8485, 0x85CF, 0x05EE, 0x05AC, 0x858D,
    0x0528, 0x8509, 0x854B, 0x056A, 0x8F9F, 0x0FBE, 0x0FFC, 0x8FDD, 0x0F78, 0x8F59, 0x8F1B, 0x0F3A,
    0x0E70, 0x8E51, 0x8E13, 0x0E32, 0x8E97, 0x0EB6, 0x0EF4, 0x8ED5, 0x0C60, 0x8C41, 0x8C03, 0x0C22,
    0x8C87, 0x0CA6, 0x0CE4, 0x8CC5, 0x8D8F, 0x0DAE, 0x0DEC, 0x8DCD, 0x0D68, 0x8D49, 0x8D0B, 0x0D2A,
    0x0840, 0x8861, 0x8823, 0x0802, 0x88A7, 0x0886, 0x08C4, 0x88E5, 0x89AF, 0x098E, 0x09CC, 0x89ED,
    0x0948, 0x8969, 0x892B, 0x090A, 0x8BBF, 0x0B9E, 0x0BDC, 0x8BFD, 0x0B58, 0x8B79, 0x8B3B, 0x0B1A,
    0x0A50, 0x8A71, 0x8A33, 0x0A12, 0x8AB7, 0x0A96, 0x0AD4, 0x8AF5, 0x9F1F, 0x1F3E, 0x1F7C, 0x9F5D,
    0x1FF8, 0x9FD9, 0x9F9B, 0x1FBA, 0x1EF0, 0x9ED1, 0x9E93, 0x1EB2, 0x9E17, 0x1E36, 0x1E74, 0x9E55,
    0x1CE0, 0x9CC1, 0x9C83, 0x1CA2, 0x9C07, 0x1C26, 0x1C64, 0x9C45, 0x9D0F, 0x1D2E, 0x1D6C, 0x9D4D,
    0x1DE8, 0x9DC9, 0x9D8B, 0x1DAA, 0x18C0, 0x98E1, 0x98A3, 0x1882, 0x9827, 0x1806, 0x1844, 0x9865,
    0x992F, 0x190E, 0x194C, 0x996D, 0x19C8, 0x99E9, 0x99AB, 0x198A, 0x9B3F, 0x1B1E, 0x1B5C, 0x9B7D,
    0x1BD8, 0x9BF9, 0x9BBB, 0x1B9A, 0x1AD0, 0x9AF1, 0x9AB3, 0x1A92, 0x9A37, 0x1A16, 0x1A54, 0x9A75,
    0x1080, 0x90A1, 0x90E3, 0x10C2, 0x9067, 0x1046, 0x1004, 0x9025, 0x916F, 0x114E, 0x110C, 0x912D,
    0x1188, 0x91A9, 0x91EB, 0x11CA, 0x937F, 0x135E, 0x131C, 0x933D, 0x1398, 0x93B9, 0x93FB, 0x13DA,
    0x1290, 0x92B1, 0x92F3, 0x12D2, 0x9277, 0x1256, 0x1214, 0x9235, 0x975F, 0x177E, 0x173C, 0x971D,
    0x17B8, 0x9799, 0x97DB, 0x17FA, 0x16B0, 0x9691, 0x96D3, 0x16F2, 0x9657, 0x1676, 0x1634, 0x9615,
    0x14A0, 0x9481, 0x94C3, 0x14E2, 0x9447, 0x1466, 0x1424, 0x9405, 0x954F, 0x156E, 0x152C, 0x950D,
    0x15A8, 0x9589, 0x95CB, 0x15EA,
];
