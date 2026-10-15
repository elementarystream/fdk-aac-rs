// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libMpegTPDec/src/tpdec_adts.h, libMpegTPDec/src/tpdec_adts.cpp.

//! Port of `tpdec_adts.h`: the ADTS header parser and the ADTS CRC.
//!
//! The code of this file is identical in the `aarch64` and `x86` reference builds.

use fdk_aac_rs_fdk::audio::ID_PCE;
use fdk_aac_rs_fdk::bitstream::{
    fdk_get_valid_bits, fdk_push_back, fdk_push_for, fdk_read_bits, FdkBitStream,
};
use fdk_aac_rs_fdk::crc::{
    fdk_crc_end_reg, fdk_crc_get_crc, fdk_crc_init, fdk_crc_reset, fdk_crc_start_reg, FdkCrcInfo,
};

use crate::tp_data::*;
use crate::tpdec_asc::{
    audio_specific_config_init, cprogram_config_compare, cprogram_config_init,
    cprogram_config_is_valid, cprogram_config_read,
};
use crate::tpdec_lib::*;

/// `ADTS_SYNCWORD`
pub const ADTS_SYNCWORD: u32 = 0xfff;
/// `ADTS_SYNCLENGTH`: in bits.
pub const ADTS_SYNCLENGTH: i32 = 12;
/// `ADTS_HEADERLENGTH`: minimum header size in bits.
pub const ADTS_HEADERLENGTH: i32 = 56;
/// `ADTS_FIXED_HEADERLENGTH`: in bits.
pub const ADTS_FIXED_HEADERLENGTH: i32 = 28;
/// `ADTS_VARIABLE_HEADERLENGTH`
pub const ADTS_VARIABLE_HEADERLENGTH: i32 = ADTS_HEADERLENGTH - ADTS_FIXED_HEADERLENGTH;
/// `ADTS_MIN_TP_BUF_SIZE`
pub const ADTS_MIN_TP_BUF_SIZE: i32 = 8191;

/// `STRUCT_ADTS_BS`: the ADTS header fields.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StructAdtsBs {
    pub mpeg_id: u8,
    pub layer: u8,
    pub protection_absent: u8,
    pub profile: u8,
    pub sample_freq_index: u8,
    pub private_bit: u8,
    pub channel_config: u8,
    pub original: u8,
    pub home: u8,
    pub copyright_id: u8,
    pub copyright_start: u8,
    pub frame_length: u16,
    pub adts_fullness: u16,
    pub num_raw_blocks: u8,
    pub num_pce_bits: u8,
}

/// `struct STRUCT_ADTS`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StructAdts {
    pub bs: StructAdtsBs,

    pub decoder_can_do_mpeg4: u8,
    pub buffer_fullnes_start_flag: u8,

    /// CRC state info
    pub crc_info: FdkCrcInfo,
    /// CRC value read from bitstream data
    pub crc_read_value: u16,
    /// distance between each raw data block. Not the same as found in the bitstream
    pub raw_data_block_dist: [u16; 4],
}

/// `HANDLE_ADTS`
pub type HandleAdts<'a> = &'a mut StructAdts;

/// `adtsRead_CrcInit(pAdts)`: initializes the CRC info.
pub fn adts_read_crc_init(p_adts: &mut StructAdts) {
    fdk_crc_init(&mut p_adts.crc_info, 0x8005, 0xFFFF, 16);
}

/// `adtsRead_CrcStartReg(pAdts, hBs, mBits)`: starts a CRC region (if the current header has
/// CRC protection). Returns the region ID (0 without protection).
pub fn adts_read_crc_start_reg(
    p_adts: &mut StructAdts,
    h_bs: &mut FdkBitStream,
    m_bits: i32,
) -> i32 {
    if p_adts.bs.protection_absent != 0 {
        return 0;
    }

    fdk_crc_start_reg(&mut p_adts.crc_info, h_bs, m_bits)
}

/// `adtsRead_CrcEndReg(pAdts, hBs, reg)`: ends a CRC region.
pub fn adts_read_crc_end_reg(p_adts: &mut StructAdts, h_bs: &mut FdkBitStream, reg: i32) {
    if p_adts.bs.protection_absent == 0 {
        fdk_crc_end_reg(&mut p_adts.crc_info, h_bs, reg);
    }
}

/// `adtsRead_CrcCheck(pAdts)`: compares the calculated CRC with the one read from the
/// bitstream.
pub fn adts_read_crc_check(p_adts: &mut StructAdts) -> TransportDecError {
    let error_status = TRANSPORTDEC_OK;

    if p_adts.bs.protection_absent != 0 {
        return TRANSPORTDEC_OK;
    }

    let crc = fdk_crc_get_crc(&p_adts.crc_info);
    if crc != p_adts.crc_read_value {
        return TRANSPORTDEC_CRC_ERROR;
    }

    error_status
}

const ADTS_LENGTH_ID: u32 = 1;
const ADTS_LENGTH_LAYER: u32 = 2;
const ADTS_LENGTH_PROTECTION_ABSENT: u32 = 1;
const ADTS_LENGTH_PROFILE: u32 = 2;
const ADTS_LENGTH_SAMPLING_FREQUENCY_INDEX: u32 = 4;
const ADTS_LENGTH_PRIVATE_BIT: u32 = 1;
const ADTS_LENGTH_CHANNEL_CONFIGURATION: u32 = 3;
const ADTS_LENGTH_ORIGINAL_COPY: u32 = 1;
const ADTS_LENGTH_HOME: u32 = 1;
const ADTS_LENGTH_COPYRIGHT_IDENTIFICATION_BIT: u32 = 1;
const ADTS_LENGTH_COPYRIGHT_IDENTIFICATION_START: u32 = 1;
const ADTS_LENGTH_FRAME_LENGTH: u32 = 13;
const ADTS_LENGTH_BUFFER_FULLNESS: u32 = 11;
const ADTS_LENGTH_NUMBER_OF_RAW_DATA_BLOCKS_IN_FRAME: u32 = 2;
const ADTS_LENGTH_CRC_CHECK: i32 = 16;

/// `adtsRead_DecodeHeader(pAdts, pAsc, bs, ignoreBufferFullness)`: parses the ADTS header at the
/// current position (after the sync word) and fills `p_asc`.
pub fn adts_read_decode_header(
    p_adts: &mut StructAdts,
    p_asc: &mut CSAudioSpecificConfig,
    h_bs: &mut FdkBitStream,
    ignore_buffer_fullness: i32,
) -> TransportDecError {
    let mut crc_reg: i32 = 0;
    let mut bs = StructAdtsBs::default();

    // Store the old PCE temporarily. Maybe we'll need it later if we have channelConfig=0 and
    // no PCE in this frame.
    let old_pce = p_asc.m_progr_config_element;

    let val_bits = fdk_get_valid_bits(h_bs) as i32 + ADTS_SYNCLENGTH;

    if val_bits < ADTS_HEADERLENGTH {
        return TRANSPORTDEC_NOT_ENOUGH_BITS;
    }

    // adts_fixed_header
    bs.mpeg_id = fdk_read_bits(h_bs, ADTS_LENGTH_ID) as u8;
    bs.layer = fdk_read_bits(h_bs, ADTS_LENGTH_LAYER) as u8;
    bs.protection_absent = fdk_read_bits(h_bs, ADTS_LENGTH_PROTECTION_ABSENT) as u8;
    bs.profile = fdk_read_bits(h_bs, ADTS_LENGTH_PROFILE) as u8;
    bs.sample_freq_index = fdk_read_bits(h_bs, ADTS_LENGTH_SAMPLING_FREQUENCY_INDEX) as u8;
    bs.private_bit = fdk_read_bits(h_bs, ADTS_LENGTH_PRIVATE_BIT) as u8;
    bs.channel_config = fdk_read_bits(h_bs, ADTS_LENGTH_CHANNEL_CONFIGURATION) as u8;
    bs.original = fdk_read_bits(h_bs, ADTS_LENGTH_ORIGINAL_COPY) as u8;
    bs.home = fdk_read_bits(h_bs, ADTS_LENGTH_HOME) as u8;

    // adts_variable_header
    bs.copyright_id = fdk_read_bits(h_bs, ADTS_LENGTH_COPYRIGHT_IDENTIFICATION_BIT) as u8;
    bs.copyright_start = fdk_read_bits(h_bs, ADTS_LENGTH_COPYRIGHT_IDENTIFICATION_START) as u8;
    bs.frame_length = fdk_read_bits(h_bs, ADTS_LENGTH_FRAME_LENGTH) as u16;
    bs.adts_fullness = fdk_read_bits(h_bs, ADTS_LENGTH_BUFFER_FULLNESS) as u16;
    bs.num_raw_blocks = fdk_read_bits(h_bs, ADTS_LENGTH_NUMBER_OF_RAW_DATA_BLOCKS_IN_FRAME) as u8;
    bs.num_pce_bits = 0;

    let mut adts_header_length = ADTS_HEADERLENGTH;

    'bail: {
        if val_bits < bs.frame_length as i32 * 8 {
            break 'bail;
        }

        fdk_crc_reset(&mut p_adts.crc_info);
        if bs.protection_absent == 0 {
            fdk_push_back(h_bs, 56); // complete fixed and variable header!
            crc_reg = fdk_crc_start_reg(&mut p_adts.crc_info, h_bs, 0);
            fdk_push_for(h_bs, 56);
        }

        if bs.protection_absent == 0 && bs.num_raw_blocks > 0 {
            if (fdk_get_valid_bits(h_bs) as i32) < bs.num_raw_blocks as i32 * 16 {
                break 'bail;
            }
            for i in 0..bs.num_raw_blocks as usize {
                p_adts.raw_data_block_dist[i] = fdk_read_bits(h_bs, 16) as u16;
                adts_header_length += 16;
            }
            // Change raw data blocks to delta values
            p_adts.raw_data_block_dist[bs.num_raw_blocks as usize] =
                (bs.frame_length as i32 - 7 - bs.num_raw_blocks as i32 * 2 - 2) as u16;
            let mut i = bs.num_raw_blocks as usize;
            while i > 0 {
                p_adts.raw_data_block_dist[i] =
                    p_adts.raw_data_block_dist[i].wrapping_sub(p_adts.raw_data_block_dist[i - 1]);
                i -= 1;
            }
        }

        // adts_error_check
        if bs.protection_absent == 0 {
            fdk_crc_end_reg(&mut p_adts.crc_info, h_bs, crc_reg);

            if (fdk_get_valid_bits(h_bs) as i32) < ADTS_LENGTH_CRC_CHECK {
                break 'bail;
            }

            let crc_check = fdk_read_bits(h_bs, ADTS_LENGTH_CRC_CHECK as u32) as u16;
            adts_header_length += ADTS_LENGTH_CRC_CHECK;

            p_adts.crc_read_value = crc_check;
            // Check header CRC in case of multiple raw data blocks
            if bs.num_raw_blocks > 0 {
                if p_adts.crc_read_value != fdk_crc_get_crc(&p_adts.crc_info) {
                    return TRANSPORTDEC_CRC_ERROR;
                }
                // Reset CRC for the upcoming raw_data_block()
                fdk_crc_reset(&mut p_adts.crc_info);
            }
        }

        // check if valid header
        if bs.layer != 0 // we only support MPEG ADTS
            || bs.sample_freq_index >= 13
        // we only support 96kHz - 7350kHz
        {
            fdk_push_for(h_bs, bs.frame_length as u32 * 8); // try again one frame later
            return TRANSPORTDEC_UNSUPPORTED_FORMAT;
        }

        // special treatment of id-bit
        if bs.mpeg_id == 0 && p_adts.decoder_can_do_mpeg4 == 0 {
            // MPEG-2 decoder cannot play MPEG-4 bitstreams

            fdk_push_for(h_bs, bs.frame_length as u32 * 8); // try again one frame later
            return TRANSPORTDEC_UNSUPPORTED_FORMAT;
        }

        if ignore_buffer_fullness == 0 {
            let cmp_buffer_fullness = bs.frame_length as i32 * 8
                + bs.adts_fullness as i32
                    * 32
                    * get_number_of_effective_channels(bs.channel_config as i32);

            // Evaluate buffer fullness
            if bs.adts_fullness != 0x7FF && p_adts.buffer_fullnes_start_flag != 0 {
                if val_bits < cmp_buffer_fullness {
                    // Condition for start of decoding is not fulfilled

                    // The current frame will not be decoded
                    fdk_push_back(h_bs, adts_header_length as u32);

                    if cmp_buffer_fullness + adts_header_length > ((8192 * 4) << 3) - 7 {
                        return TRANSPORTDEC_SYNC_ERROR;
                    } else {
                        return TRANSPORTDEC_NOT_ENOUGH_BITS;
                    }
                } else {
                    p_adts.buffer_fullnes_start_flag = 0;
                }
            }
        }

        // Get info from ADTS header
        audio_specific_config_init(p_asc);
        p_asc.m_aot = bs.profile as i32 + 1;
        p_asc.m_sampling_frequency_index = bs.sample_freq_index;
        p_asc.m_sampling_frequency = SAMPLING_RATE_TABLE[bs.sample_freq_index as usize];
        p_asc.m_channel_configuration = bs.channel_config as i8;
        p_asc.m_samples_per_frame = 1024;

        if bs.channel_config == 0 {
            let align_anchor = fdk_get_valid_bits(h_bs);
            let mut tmp_pce = CProgramConfig::zeroed();

            if fdk_read_bits(h_bs, 3) as i32 == ID_PCE {
                // Got luck! Parse the PCE
                crc_reg = adts_read_crc_start_reg(p_adts, h_bs, 0);

                cprogram_config_init(&mut tmp_pce);
                cprogram_config_read(&mut tmp_pce, h_bs, align_anchor);

                if cprogram_config_is_valid(&tmp_pce) != 0 {
                    if cprogram_config_is_valid(&old_pce) != 0 {
                        // Compare the new and the old PCE (tags ignored)
                        match cprogram_config_compare(&tmp_pce, &old_pce) {
                            0 | 1 => {
                                // Nothing to do because PCE matches the old one exactly, or
                                // channel configuration not changed. Just new metadata.
                                p_asc.m_progr_config_element = tmp_pce;
                            }
                            _ => {
                                // 2: The number of channels are identical but not the config
                                // -1: The channel configuration is completely different
                                p_asc.m_progr_config_element = old_pce;
                                fdk_push_back(h_bs, adts_header_length as u32);
                                return TRANSPORTDEC_PARSE_ERROR;
                            }
                        }
                    } else {
                        p_asc.m_progr_config_element = tmp_pce;
                    }
                } else if cprogram_config_is_valid(&old_pce) != 0 {
                    p_asc.m_progr_config_element = old_pce;
                } else {
                    fdk_push_back(h_bs, adts_header_length as u32);
                    return TRANSPORTDEC_PARSE_ERROR;
                }

                adts_read_crc_end_reg(p_adts, h_bs, crc_reg);
                let pce_bits = (align_anchor as i32).wrapping_sub(fdk_get_valid_bits(h_bs) as i32);
                adts_header_length = adts_header_length.wrapping_add(pce_bits);

                if pce_bits > align_anchor as i32 {
                    break 'bail;
                }

                // store the number of PCE bits
                bs.num_pce_bits = pce_bits as u8;
            } else {
                // No PCE in this frame! Push back the ID tag bits.
                fdk_push_back(h_bs, 3);

                // Encoders do not have to write a PCE in each frame. So if we already have a
                // valid PCE we have to use it.
                if old_pce.is_valid != 0
                    && bs.sample_freq_index == p_adts.bs.sample_freq_index // we could compare the complete fixed header (bytes) here!
                    && bs.channel_config == p_adts.bs.channel_config // == 0
                    && bs.mpeg_id == p_adts.bs.mpeg_id
                {
                    // Restore previous PCE which is still valid
                    p_asc.m_progr_config_element = old_pce;
                } else if bs.mpeg_id == 0 {
                    // If not it seems that we have a implicit channel configuration. This mode
                    // is not allowed in the context of ISO/IEC 14496-3. Skip this frame and try
                    // the next one.
                    fdk_push_for(
                        h_bs,
                        ((bs.frame_length as i32) << 3)
                            .wrapping_sub(adts_header_length)
                            .wrapping_sub(3) as u32,
                    );
                    return TRANSPORTDEC_UNSUPPORTED_FORMAT;
                }
                // else {
                //   ISO/IEC 13818-7 implicit channel mapping is allowed.
                //   So just open the box of chocolates to see what we got.
                // }
            }
        }

        // Copy bit stream data struct to persistent memory now, once we passed all sanity
        // checks above.
        p_adts.bs = bs;

        return TRANSPORTDEC_OK;
    }

    // bail:
    fdk_push_back(h_bs, adts_header_length as u32);
    TRANSPORTDEC_NOT_ENOUGH_BITS
}

/// `adtsRead_GetRawDataBlockLength(pAdts, blockNum)`: the length of the given raw data block in
/// bits, -1 if unknown.
pub fn adts_read_get_raw_data_block_length(p_adts: &StructAdts, block_num: i32) -> i32 {
    let mut length: i32;

    if p_adts.bs.num_raw_blocks == 0 {
        // aac_frame_length subtracted by the header size (7 bytes).
        length = (p_adts.bs.frame_length as i32 - 7) << 3;
        if p_adts.bs.protection_absent == 0 {
            length -= 16; // substract 16 bit CRC
        }
    } else if p_adts.bs.protection_absent != 0 {
        length = -1; // raw data block length is unknown
    } else if !(0..=3).contains(&block_num) {
        length = -1;
    } else {
        length = ((p_adts.raw_data_block_dist[block_num as usize] as i32) << 3) - 16;
    }
    if block_num == 0 && length > 0 {
        length -= p_adts.bs.num_pce_bits as i32;
    }
    length
}
