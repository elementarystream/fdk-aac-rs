// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libMpegTPDec/src/tpdec_asc.cpp.

//! Port of `tpdec_asc.cpp`: program config element (PCE) handling, AudioSpecificConfig, GA, ELD
//! and USAC specific config parsing, and the DRM SDC audio config.
//!
//! The functions are declared in `tpdec_lib.h` (and `DrmRawSdcAudioConfig_Parse()` in
//! `tpdec_drm.h`); [`crate::tpdec_lib`] and [`crate::tpdec_drm`] re-export them.
//!
//! The preprocessed translation units of the `aarch64` and `x86` reference builds are identical
//! for the code of this file (they differ only in the inline primitives of the included headers,
//! of which this file uses `fAbs(INT)`, which is the same in both builds).
//!
//! # Arrays of the caller
//!
//! `CProgramConfig_LookupElement()`, `CProgramConfig_GetChannelDescription()`,
//! `CProgramConfig_GetPceChMap()` and `CProgramConfig_GetElementTable()` write into arrays of the
//! caller with indices derived from the PCE or the caller's arguments. The port takes slices; a
//! write outside a slice is skipped and a read outside a slice gives `ID_NONE` (`elList`)
//! (PORTING.md, "Deviations from the reference").

use fdk_aac_rs_fdk::audio::*;
use fdk_aac_rs_fdk::bitbuffer::BitMem;
use fdk_aac_rs_fdk::bitstream::{
    escaped_value, fdk_byte_align, fdk_get_valid_bits, fdk_init_bit_stream, fdk_push_back,
    fdk_push_for, fdk_read_bit, fdk_read_bits, fdk_sync_cache, fdk_write_bits, FdkBitStream,
    BS_WRITER,
};
use fdk_aac_rs_fdk::crc::{
    fdk_crc_end_reg, fdk_crc_get_crc, fdk_crc_init, fdk_crc_start_reg, FdkCrcInfo,
};

use crate::tp_data::*;
use crate::tpdec_lib::*;

// ---------------------------------------------------------------------------------------------
// Channel configuration tables
// ---------------------------------------------------------------------------------------------

// The following arrays provide the IDs of the consecutive elements for each channel
// configuration. Every channel_configuration has to be finalized with ID_NONE. The values of
//   scripts/dump_table.py libMpegTPDec/src/tpdec_asc.cpp channel_configuration_N INT
// (N = 0 ... 14), written with the names of the MP4_ELEMENT_ID enumerators.
/// `channel_configuration_0`
pub static CHANNEL_CONFIGURATION_0: [Mp4ElementId; 1] = [ID_NONE];
/// `channel_configuration_1`
pub static CHANNEL_CONFIGURATION_1: [Mp4ElementId; 2] = [ID_SCE, ID_NONE];
/// `channel_configuration_2`
pub static CHANNEL_CONFIGURATION_2: [Mp4ElementId; 2] = [ID_CPE, ID_NONE];
/// `channel_configuration_3`
pub static CHANNEL_CONFIGURATION_3: [Mp4ElementId; 3] = [ID_SCE, ID_CPE, ID_NONE];
/// `channel_configuration_4`
pub static CHANNEL_CONFIGURATION_4: [Mp4ElementId; 4] = [ID_SCE, ID_CPE, ID_SCE, ID_NONE];
/// `channel_configuration_5`
pub static CHANNEL_CONFIGURATION_5: [Mp4ElementId; 4] = [ID_SCE, ID_CPE, ID_CPE, ID_NONE];
/// `channel_configuration_6`
pub static CHANNEL_CONFIGURATION_6: [Mp4ElementId; 5] = [ID_SCE, ID_CPE, ID_CPE, ID_LFE, ID_NONE];
/// `channel_configuration_7`
pub static CHANNEL_CONFIGURATION_7: [Mp4ElementId; 6] =
    [ID_SCE, ID_CPE, ID_CPE, ID_CPE, ID_LFE, ID_NONE];
/// `channel_configuration_8`
pub static CHANNEL_CONFIGURATION_8: [Mp4ElementId; 1] = [ID_NONE]; // reserved
/// `channel_configuration_9`
pub static CHANNEL_CONFIGURATION_9: [Mp4ElementId; 1] = [ID_NONE]; // reserved
/// `channel_configuration_10`
pub static CHANNEL_CONFIGURATION_10: [Mp4ElementId; 1] = [ID_NONE]; // reserved
/// `channel_configuration_11`
pub static CHANNEL_CONFIGURATION_11: [Mp4ElementId; 6] =
    [ID_SCE, ID_CPE, ID_CPE, ID_SCE, ID_LFE, ID_NONE];
/// `channel_configuration_12`
pub static CHANNEL_CONFIGURATION_12: [Mp4ElementId; 6] =
    [ID_SCE, ID_CPE, ID_CPE, ID_CPE, ID_LFE, ID_NONE];
/// `channel_configuration_13`
pub static CHANNEL_CONFIGURATION_13: [Mp4ElementId; 17] = [
    ID_SCE, ID_CPE, ID_CPE, ID_CPE, ID_CPE, ID_SCE, ID_LFE, ID_LFE, ID_SCE, ID_CPE, ID_CPE, ID_SCE,
    ID_CPE, ID_SCE, ID_SCE, ID_CPE, ID_NONE,
];
/// `channel_configuration_14`
pub static CHANNEL_CONFIGURATION_14: [Mp4ElementId; 6] =
    [ID_SCE, ID_CPE, ID_CPE, ID_LFE, ID_CPE, ID_NONE];

/// `channel_configuration_array[]` (pointers to the tables above, in the order of the C++ source)
pub static CHANNEL_CONFIGURATION_ARRAY: [&[Mp4ElementId]; 15] = [
    &CHANNEL_CONFIGURATION_0,
    &CHANNEL_CONFIGURATION_1,
    &CHANNEL_CONFIGURATION_2,
    &CHANNEL_CONFIGURATION_3,
    &CHANNEL_CONFIGURATION_4,
    &CHANNEL_CONFIGURATION_5,
    &CHANNEL_CONFIGURATION_6,
    &CHANNEL_CONFIGURATION_7,
    &CHANNEL_CONFIGURATION_8,
    &CHANNEL_CONFIGURATION_9,
    &CHANNEL_CONFIGURATION_10,
    &CHANNEL_CONFIGURATION_11,
    &CHANNEL_CONFIGURATION_12,
    &CHANNEL_CONFIGURATION_13,
    &CHANNEL_CONFIGURATION_14,
];

/// `TP_USAC_MAX_CHANNEL_CONFIGURATION_INDEX`
const TP_USAC_MAX_CHANNEL_CONFIGURATION_INDEX: usize = 13;
/// `SC_CHANNEL_CONFIG_TAB_SIZE`
const SC_CHANNEL_CONFIG_TAB_SIZE: usize = TP_USAC_MAX_CHANNEL_CONFIGURATION_INDEX + 1;

/// `SC_CHANNEL_CONFIG`: channel config structure used for sanity check.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScChannelConfig {
    /// number of channels
    pub n_ch: i8,
    /// number of SCE's
    pub n_sce: i8,
    /// number of CPE's
    pub n_cpe: i8,
    /// number of LFE's
    pub n_lfe: i8,
}

const fn scc(n_ch: i8, n_sce: i8, n_cpe: i8, n_lfe: i8) -> ScChannelConfig {
    ScChannelConfig { n_ch, n_sce, n_cpe, n_lfe }
}

/// `sc_chan_config_tab[]`: the values of
///
/// `scripts/dump_table.py libMpegTPDec/src/tpdec_asc.cpp sc_chan_config_tab SCHAR --per-line 4`
pub static SC_CHAN_CONFIG_TAB: [ScChannelConfig; SC_CHANNEL_CONFIG_TAB_SIZE] = [
    // nCh, nSCE, nCPE, nLFE,     cci
    scc(0, 0, 0, 0),  //  0
    scc(1, 1, 0, 0),  //  1
    scc(2, 0, 1, 0),  //  2
    scc(3, 1, 1, 0),  //  3
    scc(4, 2, 1, 0),  //  4
    scc(5, 1, 2, 0),  //  5
    scc(6, 1, 2, 1),  //  6
    scc(8, 1, 3, 1),  //  7
    scc(2, 2, 0, 0),  //  8
    scc(3, 1, 1, 0),  //  9
    scc(4, 0, 2, 0),  // 10
    scc(7, 2, 2, 1),  // 11
    scc(8, 1, 3, 1),  // 12
    scc(24, 6, 8, 2), // 13
];

// ---------------------------------------------------------------------------------------------
// Slice helpers for the arrays of the caller (see the module documentation)
// ---------------------------------------------------------------------------------------------

#[inline]
fn put<T: Copy>(s: &mut [T], i: usize, v: T) {
    if let Some(e) = s.get_mut(i) {
        *e = v;
    }
}

#[inline]
fn el_at(s: &[Mp4ElementId], i: usize) -> Mp4ElementId {
    match s.get(i) {
        Some(&v) => v,
        None => ID_NONE,
    }
}

// ---------------------------------------------------------------------------------------------
// CProgramConfig
// ---------------------------------------------------------------------------------------------

/// `CProgramConfig_Reset(pPce)`
pub fn cprogram_config_reset(p_pce: &mut CProgramConfig) {
    p_pce.el_counter = 0;
}

/// `CProgramConfig_Init(pPce)`
pub fn cprogram_config_init(p_pce: &mut CProgramConfig) {
    *p_pce = CProgramConfig::zeroed();
    p_pce.sampling_frequency_index = 0xf;
}

/// `CProgramConfig_IsValid(pPce)`: 1 if the PCE structure is filled correctly.
pub fn cprogram_config_is_valid(p_pce: &CProgramConfig) -> i32 {
    if p_pce.is_valid != 0 {
        1
    } else {
        0
    }
}

/// `PCE_HEIGHT_EXT_SYNC`
const PCE_HEIGHT_EXT_SYNC: u32 = 0xAC;

/// `CProgramConfig_ReadHeightExt()`: reads the extension for height info.
///
/// Returns 0 if successful, -1 if the CRC failed, -2 if the height info is invalid.
fn cprogram_config_read_height_ext(
    p_pce: &mut CProgramConfig,
    bs: &mut FdkBitStream,
    bytes_available: &mut i32,
    alignment_anchor: u32,
) -> i32 {
    let mut err = 0;
    let mut crc_info = FdkCrcInfo::default(); // CRC state info
    fdk_crc_init(&mut crc_info, 0x07, 0xFF, 8);
    let crc_reg = fdk_crc_start_reg(&mut crc_info, bs, 0);
    let start_anchor = fdk_get_valid_bits(bs);

    if start_anchor >= 24 && *bytes_available >= 3 && fdk_read_bits(bs, 8) == PCE_HEIGHT_EXT_SYNC {
        for i in 0..p_pce.num_front_channel_elements as usize {
            p_pce.front_element_height_info[i] = fdk_read_bits(bs, 2) as u8;
            if p_pce.front_element_height_info[i] as usize >= PC_NUM_HEIGHT_LAYER {
                err = -2; // height information is out of the valid range
            }
        }
        for i in 0..p_pce.num_side_channel_elements as usize {
            p_pce.side_element_height_info[i] = fdk_read_bits(bs, 2) as u8;
            if p_pce.side_element_height_info[i] as usize >= PC_NUM_HEIGHT_LAYER {
                err = -2; // height information is out of the valid range
            }
        }
        for i in 0..p_pce.num_back_channel_elements as usize {
            p_pce.back_element_height_info[i] = fdk_read_bits(bs, 2) as u8;
            if p_pce.back_element_height_info[i] as usize >= PC_NUM_HEIGHT_LAYER {
                err = -2; // height information is out of the valid range
            }
        }
        fdk_byte_align(bs, alignment_anchor);

        fdk_crc_end_reg(&mut crc_info, bs, crc_reg);
        if fdk_read_bits(bs, 8) as u16 != fdk_crc_get_crc(&crc_info) {
            // CRC failed
            err = -1;
        }
        if err != 0 {
            // Reset whole height information in case an error occured during parsing. The
            // return value ensures that pPce->isValid is set to 0 and implicit channel mapping
            // is used.
            p_pce.front_element_height_info = [0; PC_FSB_CHANNELS_MAX];
            p_pce.side_element_height_info = [0; PC_FSB_CHANNELS_MAX];
            p_pce.back_element_height_info = [0; PC_FSB_CHANNELS_MAX];
        }
    } else {
        // No valid extension data found -> restore the initial bitbuffer state
        let n = (start_anchor as i32).wrapping_sub(fdk_get_valid_bits(bs) as i32);
        fdk_push_back(bs, n as u32);
    }

    // Always report the bytes read.
    *bytes_available = bytes_available
        .wrapping_sub((start_anchor as i32).wrapping_sub(fdk_get_valid_bits(bs) as i32) >> 3);

    err
}

/// `CProgramConfig_Check(pPce)`: sanity checks for the program config element (order of the
/// elements according to ISO/IEC 13818-7:2003(E), chapter 8.5.1). Returns 0 if successful,
/// otherwise 1.
///
/// The C code walks pointers through `FrontElementIsCpe`, `SideElementIsCpe` and
/// `BackElementIsCpe`, which can continue into the following members of `CProgramConfig`; the
/// port reads the same bytes, reproduced from the C layout.
fn cprogram_config_check(p_pce: &CProgramConfig) -> i32 {
    let mut num_back_channels = [0i32; 3];
    let mut num_side_channels = [0i32; 3];
    let mut num_front_channels = [0i32; 3];
    let bytes = p_pce.as_bytes();
    let at = |i: usize| -> i32 {
        match bytes.get(i) {
            Some(&b) => b as i32,
            None => 0,
        }
    };
    let mut p_cpe_front = PCE_OFFSET_FRONT_ELEMENT_IS_CPE;
    let mut p_cpe_side = PCE_OFFSET_SIDE_ELEMENT_IS_CPE;
    let mut p_cpe_back = PCE_OFFSET_BACK_ELEMENT_IS_CPE;

    // The height values are below PC_NUM_HEIGHT_LAYER for every PCE of the library
    // (CProgramConfig_ReadHeightExt() clears invalid ones); larger values (no defined reference
    // result) are ignored here.
    for i in 0..p_pce.num_back_channel_elements as usize {
        let h = p_pce.back_element_height_info[i] as usize;
        if h < 3 {
            num_back_channels[h] += if p_pce.back_element_is_cpe[i] != 0 { 2 } else { 1 };
        }
    }
    for i in 0..p_pce.num_side_channel_elements as usize {
        let h = p_pce.side_element_height_info[i] as usize;
        if h < 3 {
            num_side_channels[h] += if p_pce.side_element_is_cpe[i] != 0 { 2 } else { 1 };
        }
    }
    for i in 0..p_pce.num_front_channel_elements as usize {
        let h = p_pce.front_element_height_info[i] as usize;
        if h < 3 {
            num_front_channels[h] += if p_pce.front_element_is_cpe[i] != 0 { 2 } else { 1 };
        }
    }

    // 0 = normal height channels, 1 = top height channels, 2 = bottom height channels
    for i in 0..3 {
        // if number of channels is odd => first element must be a SCE (front center channel)
        if num_front_channels[i] & 1 != 0 {
            let v = at(p_cpe_front);
            p_cpe_front += 1;
            if v == ID_CPE {
                return 1;
            }
            num_front_channels[i] -= 1;
        }
        while num_front_channels[i] > 0 {
            // must be CPE or paired SCE
            let v = at(p_cpe_front);
            p_cpe_front += 1;
            if v == ID_SCE {
                let v = at(p_cpe_front);
                p_cpe_front += 1;
                if v == ID_CPE {
                    return 1;
                }
            }
            num_front_channels[i] -= 2;
        }

        // in case that a top center surround channel (Ts) is transmitted the number of channels
        // can be odd
        if i != 1 {
            // number of channels must be even
            if num_side_channels[i] & 1 != 0 {
                return 1;
            }
            while num_side_channels[i] > 0 {
                // must be CPE or paired SCE
                let v = at(p_cpe_side);
                p_cpe_side += 1;
                if v == ID_SCE {
                    let v = at(p_cpe_side);
                    p_cpe_side += 1;
                    if v == ID_CPE {
                        return 1;
                    }
                }
                num_side_channels[i] -= 2;
            }
        }

        while num_back_channels[i] > 1 {
            // must be CPE or paired SCE
            let v = at(p_cpe_back);
            p_cpe_back += 1;
            if v == ID_SCE {
                let v = at(p_cpe_back);
                p_cpe_back += 1;
                if v == ID_CPE {
                    return 1;
                }
            }
            num_back_channels[i] -= 2;
        }
        // if number of channels is odd => last element must be a SCE (back center channel)
        if num_back_channels[i] != 0 {
            let v = at(p_cpe_back);
            p_cpe_back += 1;
            if v == ID_CPE {
                return 1;
            }
        }
    }

    0
}

/// `CProgramConfig_Read(pPce, bs, alignmentAnchor)`: reads a program config element.
pub fn cprogram_config_read(
    p_pce: &mut CProgramConfig,
    bs: &mut FdkBitStream,
    alignment_anchor: u32,
) {
    let mut check_element_tag_select = [[0u8; PC_FSB_CHANNELS_MAX]; 3];

    p_pce.is_valid = 1;
    p_pce.num_effective_channels = 0;
    p_pce.num_channels = 0;
    p_pce.element_instance_tag = fdk_read_bits(bs, 4) as u8;
    p_pce.profile = fdk_read_bits(bs, 2) as u8;
    p_pce.sampling_frequency_index = fdk_read_bits(bs, 4) as u8;
    p_pce.num_front_channel_elements = fdk_read_bits(bs, 4) as u8;
    p_pce.num_side_channel_elements = fdk_read_bits(bs, 4) as u8;
    p_pce.num_back_channel_elements = fdk_read_bits(bs, 4) as u8;
    p_pce.num_lfe_channel_elements = fdk_read_bits(bs, 2) as u8;
    p_pce.num_assoc_data_elements = fdk_read_bits(bs, 3) as u8;
    p_pce.num_valid_cc_elements = fdk_read_bits(bs, 4) as u8;

    p_pce.mono_mixdown_present = fdk_read_bits(bs, 1) as u8;
    if p_pce.mono_mixdown_present != 0 {
        p_pce.mono_mixdown_element_number = fdk_read_bits(bs, 4) as u8;
    }

    p_pce.stereo_mixdown_present = fdk_read_bits(bs, 1) as u8;
    if p_pce.stereo_mixdown_present != 0 {
        p_pce.stereo_mixdown_element_number = fdk_read_bits(bs, 4) as u8;
    }

    p_pce.matrix_mixdown_index_present = fdk_read_bits(bs, 1) as u8;
    if p_pce.matrix_mixdown_index_present != 0 {
        p_pce.matrix_mixdown_index = fdk_read_bits(bs, 2) as u8;
        p_pce.pseudo_surround_enable = fdk_read_bits(bs, 1) as u8;
    }

    // Check element instance tag according to ISO/IEC 13818-7:2003(E), chapter 8.2.1.1
    for i in 0..p_pce.num_front_channel_elements as usize {
        let is_cpe = fdk_read_bits(bs, 1) as u8;
        p_pce.front_element_is_cpe[i] = is_cpe;
        let tag = fdk_read_bits(bs, 4) as u8;
        p_pce.front_element_tag_select[i] = tag;
        p_pce.num_channels = p_pce.num_channels.wrapping_add(if is_cpe != 0 { 2 } else { 1 });

        let c = &mut check_element_tag_select[is_cpe as usize][tag as usize];
        if *c == 0 {
            *c = 1;
        } else {
            p_pce.is_valid = 0;
        }
    }

    for i in 0..p_pce.num_side_channel_elements as usize {
        let is_cpe = fdk_read_bits(bs, 1) as u8;
        p_pce.side_element_is_cpe[i] = is_cpe;
        let tag = fdk_read_bits(bs, 4) as u8;
        p_pce.side_element_tag_select[i] = tag;
        p_pce.num_channels = p_pce.num_channels.wrapping_add(if is_cpe != 0 { 2 } else { 1 });

        let c = &mut check_element_tag_select[is_cpe as usize][tag as usize];
        if *c == 0 {
            *c = 1;
        } else {
            p_pce.is_valid = 0;
        }
    }

    for i in 0..p_pce.num_back_channel_elements as usize {
        let is_cpe = fdk_read_bits(bs, 1) as u8;
        p_pce.back_element_is_cpe[i] = is_cpe;
        let tag = fdk_read_bits(bs, 4) as u8;
        p_pce.back_element_tag_select[i] = tag;
        p_pce.num_channels = p_pce.num_channels.wrapping_add(if is_cpe != 0 { 2 } else { 1 });

        let c = &mut check_element_tag_select[is_cpe as usize][tag as usize];
        if *c == 0 {
            *c = 1;
        } else {
            p_pce.is_valid = 0;
        }
    }

    p_pce.num_effective_channels = p_pce.num_channels;

    for i in 0..p_pce.num_lfe_channel_elements as usize {
        let tag = fdk_read_bits(bs, 4) as u8;
        p_pce.lfe_element_tag_select[i] = tag;
        p_pce.num_channels = p_pce.num_channels.wrapping_add(1);

        let c = &mut check_element_tag_select[2][tag as usize];
        if *c == 0 {
            *c = 1;
        } else {
            p_pce.is_valid = 0;
        }
    }

    for i in 0..p_pce.num_assoc_data_elements as usize {
        p_pce.assoc_data_element_tag_select[i] = fdk_read_bits(bs, 4) as u8;
    }

    for i in 0..p_pce.num_valid_cc_elements as usize {
        p_pce.cc_element_is_ind_sw[i] = fdk_read_bits(bs, 1) as u8;
        p_pce.valid_cc_element_tag_select[i] = fdk_read_bits(bs, 4) as u8;
    }

    fdk_byte_align(bs, alignment_anchor);

    p_pce.comment_field_bytes = fdk_read_bits(bs, 8) as u8;
    let mut comment_bytes = p_pce.comment_field_bytes as i32;

    // Search for height info extension and read it if available
    if cprogram_config_read_height_ext(p_pce, bs, &mut comment_bytes, alignment_anchor) != 0 {
        p_pce.is_valid = 0;
    }

    // Check order of elements according to ISO / IEC 13818 - 7:2003(E), chapter 8.5.1
    if cprogram_config_check(p_pce) != 0 {
        p_pce.is_valid = 0;
    }

    let mut i = 0;
    while i < comment_bytes {
        let text = fdk_read_bits(bs, 8) as u8;

        if (i as usize) < PC_COMMENTLENGTH {
            p_pce.comment[i as usize] = text;
        }
        i += 1;
    }
}

/// `CProgramConfig_Compare(pPce1, pPce2)`: compares two program configurations.
///
/// Returns -1 if they are completely different, 0 if completely equal, 1 if different but with
/// the same channel configuration, 2 if the channel configuration differs but the number of
/// channels is the same.
pub fn cprogram_config_compare(p_pce1: &CProgramConfig, p_pce2: &CProgramConfig) -> i32 {
    let mut result = 0; // Innocent until proven false.

    // FDKmemcmp() of the structures: they have no padding, so this is the byte comparison.
    if p_pce1.as_bytes() != p_pce2.as_bytes() {
        // Configurations are not completely equal. So look into details and analyse the
        // channel configurations:
        result = -1;

        if p_pce1.num_channels == p_pce2.num_channels {
            // Now the logic changes. We first assume to have the same channel configuration and
            // then prove if this assumption is true.
            result = 1;

            // Front channels
            if p_pce1.num_front_channel_elements != p_pce2.num_front_channel_elements {
                result = 2; // different number of front channel elements
            } else {
                let (mut num_ch1, mut num_ch2) = (0, 0);
                for el in 0..p_pce1.num_front_channel_elements as usize {
                    if p_pce1.front_element_height_info[el] != p_pce2.front_element_height_info[el]
                    {
                        result = 2; // different height info
                        break;
                    }
                    num_ch1 += if p_pce1.front_element_is_cpe[el] != 0 { 2 } else { 1 };
                    num_ch2 += if p_pce2.front_element_is_cpe[el] != 0 { 2 } else { 1 };
                }
                if num_ch1 != num_ch2 {
                    result = 2; // different number of front channels
                }
            }
            // Side channels
            if p_pce1.num_side_channel_elements != p_pce2.num_side_channel_elements {
                result = 2; // different number of side channel elements
            } else {
                let (mut num_ch1, mut num_ch2) = (0, 0);
                for el in 0..p_pce1.num_side_channel_elements as usize {
                    if p_pce1.side_element_height_info[el] != p_pce2.side_element_height_info[el] {
                        result = 2; // different height info
                        break;
                    }
                    num_ch1 += if p_pce1.side_element_is_cpe[el] != 0 { 2 } else { 1 };
                    num_ch2 += if p_pce2.side_element_is_cpe[el] != 0 { 2 } else { 1 };
                }
                if num_ch1 != num_ch2 {
                    result = 2; // different number of side channels
                }
            }
            // Back channels
            if p_pce1.num_back_channel_elements != p_pce2.num_back_channel_elements {
                result = 2; // different number of back channel elements
            } else {
                let (mut num_ch1, mut num_ch2) = (0, 0);
                for el in 0..p_pce1.num_back_channel_elements as usize {
                    if p_pce1.back_element_height_info[el] != p_pce2.back_element_height_info[el] {
                        result = 2; // different height info
                        break;
                    }
                    num_ch1 += if p_pce1.back_element_is_cpe[el] != 0 { 2 } else { 1 };
                    num_ch2 += if p_pce2.back_element_is_cpe[el] != 0 { 2 } else { 1 };
                }
                if num_ch1 != num_ch2 {
                    result = 2; // different number of back channels
                }
            }
            // LFE channels
            if p_pce1.num_lfe_channel_elements != p_pce2.num_lfe_channel_elements {
                result = 2; // different number of lfe channels
            }
            // LFEs are always SCEs so we don't need to count the channels.
        }
    }

    result
}

/// `CProgramConfig_GetDefault(pPce, channelConfig)`: a PCE that matches the predefined MPEG-4
/// channel configurations 1-14 (and 32, the 7.1 side channel configuration).
pub fn cprogram_config_get_default(p_pce: &mut CProgramConfig, channel_config: u32) {
    // Init PCE
    cprogram_config_init(p_pce);
    p_pce.profile = 1; // Set AAC LC because it is the only supported object type.

    let p = p_pce;
    // The C switch with fall-throughs, as a chain of flags.
    let (mut c12, mut c11, mut c14, mut c7, mut c6, mut c5_4, mut c3, mut c1) =
        (false, false, false, false, false, false, false, false);
    match channel_config {
        32 => {
            // 7.1 side channel configuration as defined in FDK_audio.h
            p.num_front_channel_elements = 2;
            p.front_element_is_cpe[0] = 0;
            p.front_element_is_cpe[1] = 1;
            p.num_side_channel_elements = 1;
            p.side_element_is_cpe[0] = 1;
            p.num_back_channel_elements = 1;
            p.back_element_is_cpe[0] = 1;
            p.num_lfe_channel_elements = 1;
            p.num_channels = 8;
            p.num_effective_channels = 7;
            p.is_valid = 1;
        }
        12 => c12 = true,
        11 => c11 = true,
        14 => c14 = true,
        7 => c7 = true,
        6 => c6 = true,
        5 | 4 => c5_4 = true,
        3 => c3 = true,
        1 => c1 = true,
        2 => {
            // 2/0/0.ch
            p.num_front_channel_elements = 1;
            p.front_element_is_cpe[0] = 1;
            p.num_channels = p.num_channels.wrapping_add(2);
            p.num_effective_channels = p.num_effective_channels.wrapping_add(2);
            p.is_valid = 1;
        }
        _ => {
            p.is_valid = 0; // To be explicit!
        }
    }
    if c12 {
        // 3/0/4.1ch surround back
        p.back_element_is_cpe[1] = 1;
        p.num_channels = p.num_channels.wrapping_add(1);
        p.num_effective_channels = p.num_effective_channels.wrapping_add(1);
        c11 = true;
    }
    if c11 {
        // 3/0/3.1ch
        p.num_front_channel_elements = p.num_front_channel_elements.wrapping_add(2);
        p.front_element_is_cpe[0] = 0;
        p.front_element_is_cpe[1] = 1;
        p.num_back_channel_elements = p.num_back_channel_elements.wrapping_add(2);
        p.back_element_is_cpe[0] = 1;
        // pPce->BackElementIsCpe[1] += 0;
        p.num_lfe_channel_elements = p.num_lfe_channel_elements.wrapping_add(1);
        p.num_channels = p.num_channels.wrapping_add(7);
        p.num_effective_channels = p.num_effective_channels.wrapping_add(6);
        p.is_valid = 1;
    }
    if c14 {
        // 2/0/0-3/0/2-0.1ch front height
        p.front_element_height_info[2] = 1; // Top speaker
        c7 = true;
    }
    if c7 {
        // 5/0/2.1ch front
        p.num_front_channel_elements = p.num_front_channel_elements.wrapping_add(1);
        p.front_element_is_cpe[2] = 1;
        p.num_channels = p.num_channels.wrapping_add(2);
        p.num_effective_channels = p.num_effective_channels.wrapping_add(2);
        c6 = true;
    }
    if c6 {
        // 3/0/2.1ch
        p.num_lfe_channel_elements = p.num_lfe_channel_elements.wrapping_add(1);
        p.num_channels = p.num_channels.wrapping_add(1);
        c5_4 = true;
    }
    if c5_4 {
        // 3/0/2.0ch, 3/0/1.0ch
        let gt4 = channel_config > 4;
        p.num_back_channel_elements = p.num_back_channel_elements.wrapping_add(1);
        p.back_element_is_cpe[0] = if gt4 { 1 } else { 0 };
        p.num_channels = p.num_channels.wrapping_add(if gt4 { 2 } else { 1 });
        p.num_effective_channels = p.num_effective_channels.wrapping_add(if gt4 { 2 } else { 1 });
        c3 = true;
    }
    if c3 {
        // 3/0/0.0ch
        p.num_front_channel_elements = p.num_front_channel_elements.wrapping_add(1);
        p.front_element_is_cpe[1] = 1;
        p.num_channels = p.num_channels.wrapping_add(2);
        p.num_effective_channels = p.num_effective_channels.wrapping_add(2);
        c1 = true;
    }
    if c1 {
        // 1/0/0.0ch
        p.num_front_channel_elements = p.num_front_channel_elements.wrapping_add(1);
        p.front_element_is_cpe[0] = 0;
        p.num_channels = p.num_channels.wrapping_add(1);
        p.num_effective_channels = p.num_effective_channels.wrapping_add(1);
        p.is_valid = 1;
    }

    if p.is_valid != 0 {
        // Create valid element instance tags
        let (mut el_tag_sce, mut el_tag_cpe) = (0u8, 0u8);

        for el in 0..p.num_front_channel_elements as usize {
            p.front_element_tag_select[el] = if p.front_element_is_cpe[el] != 0 {
                el_tag_cpe += 1;
                el_tag_cpe - 1
            } else {
                el_tag_sce += 1;
                el_tag_sce - 1
            };
        }
        for el in 0..p.num_side_channel_elements as usize {
            p.side_element_tag_select[el] = if p.side_element_is_cpe[el] != 0 {
                el_tag_cpe += 1;
                el_tag_cpe - 1
            } else {
                el_tag_sce += 1;
                el_tag_sce - 1
            };
        }
        for el in 0..p.num_back_channel_elements as usize {
            p.back_element_tag_select[el] = if p.back_element_is_cpe[el] != 0 {
                el_tag_cpe += 1;
                el_tag_cpe - 1
            } else {
                el_tag_sce += 1;
                el_tag_sce - 1
            };
        }
        el_tag_sce = 0;
        for el in 0..p.num_lfe_channel_elements as usize {
            p.lfe_element_tag_select[el] = el_tag_sce;
            el_tag_sce += 1;
        }
    }
}

/// `getImplicitAudioChannelTypeAndIndex()`: the implicit audio channel type and index for the
/// given channel configuration (1..14) and MPEG ordered channel index. Leaves the outputs
/// unchanged for combinations without a definition.
fn get_implicit_audio_channel_type_and_index(
    ch_type: &mut AudioChannelType,
    ch_index: &mut u8,
    channel_config: u32,
    index: u32,
) {
    if index < 3 {
        *ch_type = ACT_FRONT;
        *ch_index = index as u8;
    } else {
        match channel_config {
            4..=6 => {
                // SCE, CPE, SCE / SCE, CPE, CPE / SCE, CPE, CPE, LFE
                match index {
                    3 | 4 => {
                        *ch_type = ACT_BACK;
                        *ch_index = (index - 3) as u8;
                    }
                    5 => {
                        *ch_type = ACT_LFE;
                        *ch_index = 0;
                    }
                    _ => {}
                }
            }
            7 => {
                // SCE,CPE,CPE,CPE,LFE
                match index {
                    3 | 4 => {
                        *ch_type = ACT_FRONT;
                        *ch_index = index as u8;
                    }
                    5 | 6 => {
                        *ch_type = ACT_BACK;
                        *ch_index = (index - 5) as u8;
                    }
                    7 => {
                        *ch_type = ACT_LFE;
                        *ch_index = 0;
                    }
                    _ => {}
                }
            }
            11 => {
                // SCE,CPE,CPE,SCE,LFE
                if index < 6 {
                    *ch_type = ACT_BACK;
                    *ch_index = (index - 3) as u8;
                } else {
                    *ch_type = ACT_LFE;
                    *ch_index = 0;
                }
            }
            12 => {
                // SCE,CPE,CPE,CPE,LFE
                if index < 7 {
                    *ch_type = ACT_BACK;
                    *ch_index = (index - 3) as u8;
                } else {
                    *ch_type = ACT_LFE;
                    *ch_index = 0;
                }
            }
            14 => {
                // SCE,CPE,CPE,LFE,CPE
                match index {
                    3 | 4 => {
                        *ch_type = ACT_BACK;
                        *ch_index = (index - 3) as u8;
                    }
                    5 => {
                        *ch_type = ACT_LFE;
                        *ch_index = 0;
                    }
                    6 | 7 => {
                        *ch_type = ACT_FRONT_TOP;
                        *ch_index = (index - 6) as u8; // handle the top layer independently
                    }
                    _ => {}
                }
            }
            _ => {
                *ch_type = ACT_NONE;
            }
        }
    }
}

/// `getImplicitAudioChannelTypeAndIndex(&chType[idx], &chIndex[idx], ...)` on the arrays of the
/// caller.
fn implicit_at(
    ch_type: &mut [AudioChannelType],
    ch_index: &mut [u8],
    channel_config: u32,
    idx: u32,
) {
    let i = idx as usize;
    // C passes pointers to both elements; the port uses scratch values for elements outside the
    // slices (the writes to them are dropped).
    let mut t = ch_type.get(i).copied().unwrap_or(0);
    let mut x = ch_index.get(i).copied().unwrap_or(0);
    get_implicit_audio_channel_type_and_index(&mut t, &mut x, channel_config, idx);
    put(ch_type, i, t);
    put(ch_index, i, x);
}

/// `CProgramConfig_LookupElement(pPce, channelConfig, tag, channelIdx, chMapping, chType,
/// chIndex, chDescrLen, elMapping, elList, elType)`: looks up an element of the bitstream in the
/// PCE (or the implicit channel configuration) and fills the channel mapping. Returns 1 if the
/// element belongs to the program, otherwise 0.
#[allow(clippy::too_many_arguments)]
pub fn cprogram_config_lookup_element(
    p_pce: &mut CProgramConfig,
    mut channel_config: u32,
    tag: u32,
    channel_idx: u32,
    ch_mapping: &mut [u8],
    ch_type: &mut [AudioChannelType],
    ch_index: &mut [u8],
    ch_descr_len: u32,
    el_mapping: &mut u8,
    el_list: &mut [Mp4ElementId],
    el_type: Mp4ElementId,
) -> i32 {
    if channel_config > 0 {
        // Constant channel mapping must have been set during initialization.
        if is_channel_element(el_type) {
            *el_mapping = p_pce.el_counter;
            if el_at(el_list, p_pce.el_counter as usize) != el_type
                && !is_usac_channel_element(el_type)
            {
                // Not in the list
                if channel_config == 2 && el_type == ID_SCE {
                    // This scenario occurs with HE-AAC v2 streams of buggy encoders. In other
                    // decoder implementations decoding of this kind of streams is desired.
                    channel_config = 1;
                } else if el_at(el_list, p_pce.el_counter as usize) == ID_LFE && el_type == ID_SCE {
                    // Decode bitstreams which wrongly use ID_SCE instead of ID_LFE element type.
                } else {
                    return 0;
                }
            }
            // Assume all front channels
            implicit_at(ch_type, ch_index, channel_config, channel_idx);
            if el_type == ID_CPE || el_type == ID_USAC_CPE {
                let i = channel_idx as usize;
                let t = ch_type.get(i).copied().unwrap_or(0);
                let x = ch_index.get(i).copied().unwrap_or(0);
                put(ch_type, i.wrapping_add(1), t);
                put(ch_index, i.wrapping_add(1), x.wrapping_add(1));
            }
            p_pce.el_counter = p_pce.el_counter.wrapping_add(1);
        }
        // Accept all non-channel elements, too.
        1
    } else {
        if p_pce.is_valid == 0 || p_pce.num_channels as u32 > ch_descr_len {
            // Implicit channel mapping.
            if is_usac_channel_element(el_type) {
                *el_mapping = p_pce.el_counter;
                p_pce.el_counter = p_pce.el_counter.wrapping_add(1);
            } else if is_mp4_channel_element(el_type) {
                // Store all channel element IDs
                put(el_list, p_pce.el_counter as usize, el_type);
                *el_mapping = p_pce.el_counter;
                p_pce.el_counter = p_pce.el_counter.wrapping_add(1);
            }
        } else {
            // Accept the additional channel(s), only if the tag is in the lists
            let mut is_cpe = 0u8;
            // Element counter
            let mut ec = [0i32; PC_NUM_HEIGHT_LAYER];
            // Channel counters
            let mut cc = [0i32; PC_NUM_HEIGHT_LAYER];
            let mut fc = [0i32; PC_NUM_HEIGHT_LAYER]; // front channel counter
            let mut sc = [0i32; PC_NUM_HEIGHT_LAYER]; // side channel counter
            let mut bc = [0i32; PC_NUM_HEIGHT_LAYER]; // back channel counter
            let mut lc = 0i32; // lfe channel counter

            // General MPEG (PCE) composition rules:
            // - Over all:
            //     <normal height channels><top height channels><bottom height channels>
            // - Within each height layer:
            //     <front channels><side channels><back channels>
            // - Exception:
            //     The LFE channels have no height info and thus they are arranged at the very
            //     end of the normal height layer channels.

            // Counts the elements and channels of all height layers below `height_layer`
            // (`for (h = heightLayer - 1; h >= 0; h -= 1)`).
            let count_lower =
                |p: &CProgramConfig, height_layer: i32, el_idx: &mut i32, ch_idx: &mut i32| {
                    let mut h = height_layer - 1;
                    while h >= 0 {
                        // Count front channels/elements
                        for el in 0..p.num_front_channel_elements as usize {
                            if p.front_element_height_info[el] as i32 == h {
                                *el_idx += 1;
                                *ch_idx += if p.front_element_is_cpe[el] != 0 { 2 } else { 1 };
                            }
                        }
                        // Count side channels/elements
                        for el in 0..p.num_side_channel_elements as usize {
                            if p.side_element_height_info[el] as i32 == h {
                                *el_idx += 1;
                                *ch_idx += if p.side_element_is_cpe[el] != 0 { 2 } else { 1 };
                            }
                        }
                        // Count back channels/elements
                        for el in 0..p.num_back_channel_elements as usize {
                            if p.back_element_height_info[el] as i32 == h {
                                *el_idx += 1;
                                *ch_idx += if p.back_element_is_cpe[el] != 0 { 2 } else { 1 };
                            }
                        }
                        if h == 0 {
                            // normal height (LFE channels belong to the normal height layer)
                            *el_idx += p.num_lfe_channel_elements as i32;
                            *ch_idx += p.num_lfe_channel_elements as i32;
                        }
                        h -= 1;
                    }
                };

            // Writes the mapping of the found element.
            let mut map = |ch_idx: i32, a_ch_type: AudioChannelType, idx0: i32, is_cpe: u8| {
                let c = ch_idx as usize;
                put(ch_mapping, c, channel_idx as u8);
                put(ch_type, c, a_ch_type);
                put(ch_index, c, idx0 as u8);
                if is_cpe != 0 {
                    put(ch_mapping, c.wrapping_add(1), channel_idx.wrapping_add(1) as u8);
                    put(ch_type, c.wrapping_add(1), a_ch_type);
                    put(ch_index, c.wrapping_add(1), (idx0 + 1) as u8);
                }
            };

            // The height values index arrays of PC_NUM_HEIGHT_LAYER entries; they are always
            // below PC_NUM_HEIGHT_LAYER (see cprogram_config_check()). `hl()` keeps the port
            // from panicking on other values.
            let hl = |h: u8| -> usize { (h as usize).min(PC_NUM_HEIGHT_LAYER - 1) };

            match el_type {
                ID_CPE | ID_SCE => {
                    if el_type == ID_CPE {
                        is_cpe = 1;
                    }
                    // search in front channels
                    for i in 0..p_pce.num_front_channel_elements as usize {
                        let height_layer = p_pce.front_element_height_info[i];
                        let h = hl(height_layer);
                        if is_cpe == p_pce.front_element_is_cpe[i]
                            && p_pce.front_element_tag_select[i] as u32 == tag
                        {
                            let (mut el_idx, mut ch_idx) = (ec[h], cc[h]);
                            let a_ch_type = ((height_layer as i32) << 4) | ACT_FRONT;
                            count_lower(p_pce, height_layer as i32, &mut el_idx, &mut ch_idx);
                            map(ch_idx, a_ch_type, fc[h], is_cpe);
                            *el_mapping = el_idx as u8;
                            return 1;
                        }
                        ec[h] += 1;
                        if p_pce.front_element_is_cpe[i] != 0 {
                            cc[h] += 2;
                            fc[h] += 2;
                        } else {
                            cc[h] += 1;
                            fc[h] += 1;
                        }
                    }
                    // search in side channels
                    for i in 0..p_pce.num_side_channel_elements as usize {
                        let height_layer = p_pce.side_element_height_info[i];
                        let h = hl(height_layer);
                        if is_cpe == p_pce.side_element_is_cpe[i]
                            && p_pce.side_element_tag_select[i] as u32 == tag
                        {
                            let (mut el_idx, mut ch_idx) = (ec[h], cc[h]);
                            let a_ch_type = ((height_layer as i32) << 4) | ACT_SIDE;
                            count_lower(p_pce, height_layer as i32, &mut el_idx, &mut ch_idx);
                            map(ch_idx, a_ch_type, sc[h], is_cpe);
                            *el_mapping = el_idx as u8;
                            return 1;
                        }
                        ec[h] += 1;
                        if p_pce.side_element_is_cpe[i] != 0 {
                            cc[h] += 2;
                            sc[h] += 2;
                        } else {
                            cc[h] += 1;
                            sc[h] += 1;
                        }
                    }
                    // search in back channels
                    for i in 0..p_pce.num_back_channel_elements as usize {
                        let height_layer = p_pce.back_element_height_info[i];
                        let h = hl(height_layer);
                        if is_cpe == p_pce.back_element_is_cpe[i]
                            && p_pce.back_element_tag_select[i] as u32 == tag
                        {
                            let (mut el_idx, mut ch_idx) = (ec[h], cc[h]);
                            let a_ch_type = ((height_layer as i32) << 4) | ACT_BACK;
                            count_lower(p_pce, height_layer as i32, &mut el_idx, &mut ch_idx);
                            map(ch_idx, a_ch_type, bc[h], is_cpe);
                            *el_mapping = el_idx as u8;
                            return 1;
                        }
                        ec[h] += 1;
                        if p_pce.back_element_is_cpe[i] != 0 {
                            cc[h] += 2;
                            bc[h] += 2;
                        } else {
                            cc[h] += 1;
                            bc[h] += 1;
                        }
                    }
                }

                ID_LFE => {
                    // Unfortunately we have to go through all normal height layer elements to
                    // get the position of the LFE channels. Start with counting the front
                    // channels/elements at normal height
                    for i in 0..p_pce.num_front_channel_elements as usize {
                        let h = hl(p_pce.front_element_height_info[i]);
                        ec[h] += 1;
                        cc[h] += if p_pce.front_element_is_cpe[i] != 0 { 2 } else { 1 };
                    }
                    // Count side channels/elements at normal height
                    for i in 0..p_pce.num_side_channel_elements as usize {
                        let h = hl(p_pce.side_element_height_info[i]);
                        ec[h] += 1;
                        cc[h] += if p_pce.side_element_is_cpe[i] != 0 { 2 } else { 1 };
                    }
                    // Count back channels/elements at normal height
                    for i in 0..p_pce.num_back_channel_elements as usize {
                        let h = hl(p_pce.back_element_height_info[i]);
                        ec[h] += 1;
                        cc[h] += if p_pce.back_element_is_cpe[i] != 0 { 2 } else { 1 };
                    }

                    // search in lfe channels
                    for i in 0..p_pce.num_lfe_channel_elements as usize {
                        let el_idx = ec[0]; // LFE channels belong to the normal height layer
                        let ch_idx = cc[0];
                        if p_pce.lfe_element_tag_select[i] as u32 == tag {
                            put(ch_mapping, ch_idx as usize, channel_idx as u8);
                            *el_mapping = el_idx as u8;
                            put(ch_type, ch_idx as usize, ACT_LFE);
                            put(ch_index, ch_idx as usize, lc as u8);
                            return 1;
                        }
                        ec[0] += 1;
                        cc[0] += 1;
                        lc += 1;
                    }
                }

                // Non audio elements
                ID_CCE => {
                    // search in cce channels
                    for i in 0..p_pce.num_valid_cc_elements as usize {
                        if p_pce.valid_cc_element_tag_select[i] as u32 == tag {
                            return 1;
                        }
                    }
                }
                ID_DSE => {
                    // search associated data elements
                    for i in 0..p_pce.num_assoc_data_elements as usize {
                        if p_pce.assoc_data_element_tag_select[i] as u32 == tag {
                            return 1;
                        }
                    }
                }
                _ => return 0,
            }
            return 0; // not found in any list
        }
        1
    }
}

const SPEAKER_PLANE_NORMAL: i32 = 0;
const SPEAKER_PLANE_TOP: usize = 1;
const SPEAKER_PLANE_BOTTOM: i32 = 2;

/// `CProgramConfig_GetChannelDescription(chConfig, pPce, chType, chIndex)`: the channel types
/// and indices of a channel configuration (`ch_config > 0`) in MPEG canonical order, or of a PCE
/// (`ch_config == 0`).
pub fn cprogram_config_get_channel_description(
    ch_config: u32,
    p_pce: Option<&CProgramConfig>,
    ch_type: &mut [AudioChannelType],
    ch_index: &mut [u8],
) {
    if ch_config == 0 && p_pce.is_some() {
        let p = match p_pce {
            Some(p) => p,
            None => return,
        };
        if p.is_valid != 0 {
            let mut ch_idx = 0usize;
            let mut spk_plane = SPEAKER_PLANE_NORMAL;
            while spk_plane <= SPEAKER_PLANE_BOTTOM {
                let mut grp_ch_idx = 0u8;
                for el_idx in 0..p.num_front_channel_elements as usize {
                    if p.front_element_height_info[el_idx] as i32 == spk_plane {
                        put(ch_type, ch_idx, (spk_plane << 4) | ACT_FRONT);
                        put(ch_index, ch_idx, grp_ch_idx);
                        ch_idx += 1;
                        grp_ch_idx = grp_ch_idx.wrapping_add(1);
                        if p.front_element_is_cpe[el_idx] != 0 {
                            put(ch_type, ch_idx, (spk_plane << 4) | ACT_FRONT);
                            put(ch_index, ch_idx, grp_ch_idx);
                            ch_idx += 1;
                            grp_ch_idx = grp_ch_idx.wrapping_add(1);
                        }
                    }
                }
                grp_ch_idx = 0;
                for el_idx in 0..p.num_side_channel_elements as usize {
                    if p.side_element_height_info[el_idx] as i32 == spk_plane {
                        put(ch_type, ch_idx, (spk_plane << 4) | ACT_SIDE);
                        put(ch_index, ch_idx, grp_ch_idx);
                        ch_idx += 1;
                        grp_ch_idx = grp_ch_idx.wrapping_add(1);
                        if p.side_element_is_cpe[el_idx] != 0 {
                            put(ch_type, ch_idx, (spk_plane << 4) | ACT_SIDE);
                            put(ch_index, ch_idx, grp_ch_idx);
                            ch_idx += 1;
                            grp_ch_idx = grp_ch_idx.wrapping_add(1);
                        }
                    }
                }
                grp_ch_idx = 0;
                for el_idx in 0..p.num_back_channel_elements as usize {
                    if p.back_element_height_info[el_idx] as i32 == spk_plane {
                        put(ch_type, ch_idx, (spk_plane << 4) | ACT_BACK);
                        put(ch_index, ch_idx, grp_ch_idx);
                        ch_idx += 1;
                        grp_ch_idx = grp_ch_idx.wrapping_add(1);
                        if p.back_element_is_cpe[el_idx] != 0 {
                            put(ch_type, ch_idx, (spk_plane << 4) | ACT_BACK);
                            put(ch_index, ch_idx, grp_ch_idx);
                            ch_idx += 1;
                            grp_ch_idx = grp_ch_idx.wrapping_add(1);
                        }
                    }
                }
                grp_ch_idx = 0;
                if spk_plane == SPEAKER_PLANE_NORMAL {
                    for _ in 0..p.num_lfe_channel_elements {
                        put(ch_type, ch_idx, ACT_LFE);
                        put(ch_index, ch_idx, grp_ch_idx);
                        ch_idx += 1;
                        grp_ch_idx = grp_ch_idx.wrapping_add(1);
                    }
                }
                spk_plane += 1;
            }
        }
    } else {
        let n = get_number_of_total_channels(ch_config as i32);
        let mut ch_idx = 0;
        while ch_idx < n {
            implicit_at(ch_type, ch_index, ch_config, ch_idx as u32);
            ch_idx += 1;
        }
    }
}

/// `CProgramConfig_GetPceChMap(pPce, pceChMap, pceChMapLen)`: the channel map of a PCE (the
/// order of the channels of the height layers within the MPEG canonical order). Returns 0 on
/// success, -1 if the PCE has more than `pce_ch_map_len` channels.
pub fn cprogram_config_get_pce_ch_map(
    p_pce: &CProgramConfig,
    pce_ch_map: &mut [u8],
    pce_ch_map_len: u32,
) -> i32 {
    let n_elements = [
        p_pce.num_front_channel_elements,
        p_pce.num_side_channel_elements,
        p_pce.num_back_channel_elements,
    ];
    let el_height: [&[u8; PC_FSB_CHANNELS_MAX]; 3] = [
        &p_pce.front_element_height_info,
        &p_pce.side_element_height_info,
        &p_pce.back_element_height_info,
    ];
    let el_is_cpe: [&[u8; PC_FSB_CHANNELS_MAX]; 3] =
        [&p_pce.front_element_is_cpe, &p_pce.side_element_is_cpe, &p_pce.back_element_is_cpe];
    // Init counter:
    let mut tot_ch = [0u32; 3];
    let mut num_ch = [[0u32; 4]; 3];

    // Analyse PCE:
    let mut grp: u32;
    for plane in 0..=SPEAKER_PLANE_BOTTOM as usize {
        grp = 0;
        while grp < 3 {
            // front, side, back
            for el in 0..n_elements[grp as usize] as usize {
                if el_height[grp as usize][el] as usize == plane {
                    let el_ch = if el_is_cpe[grp as usize][el] != 0 { 2 } else { 1 };
                    num_ch[plane][grp as usize] += el_ch;
                    tot_ch[plane] += el_ch;
                }
            }
            grp += 1;
        }
        if plane == SPEAKER_PLANE_NORMAL as usize {
            // grp == 3 here: the LFE group
            let el_ch = p_pce.num_lfe_channel_elements as u32;
            num_ch[plane][grp as usize] += el_ch;
            tot_ch[plane] += el_ch;
        }
    }
    // Sanity checks:
    let mut ch_idx = tot_ch[SPEAKER_PLANE_NORMAL as usize]
        .wrapping_add(tot_ch[SPEAKER_PLANE_TOP])
        .wrapping_add(tot_ch[SPEAKER_PLANE_BOTTOM as usize]);
    if ch_idx > pce_ch_map_len {
        return -1;
    }

    // Create map:
    let mut offset: u32 = 0;
    grp = 0;
    let mut grp_thresh = num_ch[SPEAKER_PLANE_NORMAL as usize][grp as usize];
    ch_idx = 0;
    while ch_idx < tot_ch[SPEAKER_PLANE_NORMAL as usize] {
        while ch_idx >= grp_thresh && grp < 3 {
            offset += num_ch[1][grp as usize] + num_ch[2][grp as usize];
            grp += 1;
            grp_thresh += num_ch[SPEAKER_PLANE_NORMAL as usize][grp as usize];
        }
        put(pce_ch_map, ch_idx as usize, ch_idx.wrapping_add(offset) as u8);
        ch_idx += 1;
    }
    offset = 0;
    for grp in 0..4 {
        // front, side, back and lfe
        offset += num_ch[SPEAKER_PLANE_NORMAL as usize][grp];
        for plane in SPEAKER_PLANE_TOP..=SPEAKER_PLANE_BOTTOM as usize {
            for _ in 0..num_ch[plane][grp] {
                put(pce_ch_map, ch_idx as usize, offset as u8);
                ch_idx += 1;
                offset += 1;
            }
        }
    }
    0
}

/// `CProgramConfig_GetElementTable(pPce, elList, elListSize, pChMapIdx)`: the element list of a
/// PCE and the matching channel configuration index (0 if none). Returns the number of
/// elements, 0 if they do not fit into `el_list_size` or the PCE has no channels.
pub fn cprogram_config_get_element_table(
    p_pce: &CProgramConfig,
    el_list: &mut [Mp4ElementId],
    el_list_size: i32,
    p_ch_map_idx: &mut u8,
) -> i32 {
    let mut el: usize = 0;

    *p_ch_map_idx = 0;

    if el_list_size
        < p_pce.num_front_channel_elements as i32
            + p_pce.num_side_channel_elements as i32
            + p_pce.num_back_channel_elements as i32
            + p_pce.num_lfe_channel_elements as i32
        || p_pce.num_channels == 0
    {
        return 0;
    }

    for i in 0..p_pce.num_front_channel_elements as usize {
        put(el_list, el, if p_pce.front_element_is_cpe[i] != 0 { ID_CPE } else { ID_SCE });
        el += 1;
    }

    for i in 0..p_pce.num_side_channel_elements as usize {
        put(el_list, el, if p_pce.side_element_is_cpe[i] != 0 { ID_CPE } else { ID_SCE });
        el += 1;
    }

    for i in 0..p_pce.num_back_channel_elements as usize {
        put(el_list, el, if p_pce.back_element_is_cpe[i] != 0 { ID_CPE } else { ID_SCE });
        el += 1;
    }

    for _ in 0..p_pce.num_lfe_channel_elements {
        put(el_list, el, ID_LFE);
        el += 1;
    }

    // Find an corresponding channel configuration if possible
    match p_pce.num_channels {
        1 | 2 => {
            // One and two channels have no alternatives.
            *p_ch_map_idx = p_pce.num_channels;
        }
        3..=6 => {
            // Test if the number of channels can be used as channel config:
            let mut tmp_pce = CProgramConfig::zeroed();
            // Create a PCE for the config to test ...
            cprogram_config_get_default(&mut tmp_pce, p_pce.num_channels as u32);
            // ... and compare it with the given one.
            *p_ch_map_idx = if cprogram_config_compare(p_pce, &tmp_pce) & 0xE == 0 {
                p_pce.num_channels
            } else {
                0
            };
        }
        7 => {
            let mut tmp_pce = CProgramConfig::zeroed();
            // Create a PCE for the config to test ...
            cprogram_config_get_default(&mut tmp_pce, 11);
            // ... and compare it with the given one.
            *p_ch_map_idx =
                if cprogram_config_compare(p_pce, &tmp_pce) & 0xE == 0 { 11 } else { 0 };
            // If compare result is 0 or 1 we can be sure that it is channel config 11.
        }
        8 => {
            // Try the four possible 7.1ch configurations. One after the other.
            let test_cfg: [u8; 4] = [32, 14, 12, 7];
            let mut tmp_pce = CProgramConfig::zeroed();
            for &cfg in test_cfg.iter() {
                // Create a PCE for the config to test ...
                cprogram_config_get_default(&mut tmp_pce, cfg as u32);
                // ... and compare it with the given one.
                if cprogram_config_compare(p_pce, &tmp_pce) & 0xE == 0 {
                    // If the compare result is 0 or 1 than the two channel configurations match.
                    // Explicit mapping of 7.1 side channel configuration to 7.1 rear channel
                    // mapping.
                    *p_ch_map_idx = if cfg == 32 { 12 } else { cfg };
                }
            }
        }
        _ => {
            // The PCE does not match any predefined channel configuration.
            *p_ch_map_idx = 0;
        }
    }

    el as i32
}

// ---------------------------------------------------------------------------------------------
// AudioSpecificConfig helpers
// ---------------------------------------------------------------------------------------------

/// `getAOT(bs)`
fn get_aot(bs: &mut FdkBitStream) -> AudioObjectType {
    let mut tmp = fdk_read_bits(bs, 5) as i32;
    if tmp == AOT_ESCAPE {
        let tmp2 = fdk_read_bits(bs, 6) as i32;
        tmp = 32 + tmp2;
    }

    tmp
}

/// `getSampleRate(bs, index, nBits)`: reads a sampling frequency index (and an explicit
/// frequency for the escape value). Returns 0 (and leaves `index` unchanged) if fewer than 24
/// bits are left for the explicit frequency.
fn get_sample_rate(bs: &mut FdkBitStream, index: &mut u8, n_bits: u32) -> i32 {
    let sample_rate: i32;

    let idx = fdk_read_bits(bs, n_bits) as i32;
    if idx == (1 << n_bits) - 1 {
        if fdk_get_valid_bits(bs) < 24 {
            return 0;
        }
        sample_rate = fdk_read_bits(bs, 24) as i32;
    } else {
        // n_bits is 4 or 5: idx is at most 30.
        sample_rate = sampling_rate_table(idx as usize) as i32;
    }

    *index = idx as u8;

    sample_rate
}

/// `GaSpecificConfig_Parse(&asc->m_sc.m_gaSpecificConfig, asc, bs, ascStartAnchor)`
fn ga_specific_config_parse(
    asc: &mut CSAudioSpecificConfig,
    bs: &mut FdkBitStream,
    asc_start_anchor: u32,
) -> TransportDecError {
    let error_status = TRANSPORTDEC_OK;

    asc.m_sc.m_ga_specific_config.m_frame_length_flag = fdk_read_bits(bs, 1);

    asc.m_sc.m_ga_specific_config.m_depends_on_core_coder = fdk_read_bits(bs, 1);

    if asc.m_sc.m_ga_specific_config.m_depends_on_core_coder != 0 {
        asc.m_sc.m_ga_specific_config.m_core_coder_delay = fdk_read_bits(bs, 14);
    }

    asc.m_sc.m_ga_specific_config.m_extension_flag = fdk_read_bits(bs, 1);

    if asc.m_channel_configuration == 0 {
        cprogram_config_read(&mut asc.m_progr_config_element, bs, asc_start_anchor);
    }

    if asc.m_aot == AOT_AAC_SCAL || asc.m_aot == AOT_ER_AAC_SCAL {
        asc.m_sc.m_ga_specific_config.m_layer = fdk_read_bits(bs, 3);
    }

    if asc.m_sc.m_ga_specific_config.m_extension_flag != 0 {
        if asc.m_aot == AOT_ER_BSAC {
            asc.m_sc.m_ga_specific_config.m_num_of_sub_frame = fdk_read_bits(bs, 5);
            asc.m_sc.m_ga_specific_config.m_layer_length = fdk_read_bits(bs, 11);
        }

        if asc.m_aot == AOT_ER_AAC_LC
            || asc.m_aot == AOT_ER_AAC_LTP
            || asc.m_aot == AOT_ER_AAC_SCAL
            || asc.m_aot == AOT_ER_AAC_LD
        {
            asc.m_vcb11_flag = fdk_read_bits(bs, 1) as i8; // aacSectionDataResilienceFlag
            asc.m_rvlc_flag = fdk_read_bits(bs, 1) as i8; // aacScalefactorDataResilienceFlag
            asc.m_hcr_flag = fdk_read_bits(bs, 1) as i8; // aacSpectralDataResilienceFlag
        }

        asc.m_sc.m_ga_specific_config.m_extension_flag3 = fdk_read_bits(bs, 1);
    }
    error_status
}

/// `skipSbrHeader(hBs, isUsac)`: dummy parse of `SbrDfltHeader()`. Returns the number of bits
/// skipped.
fn skip_sbr_header(h_bs: &mut FdkBitStream, is_usac: i32) -> i32 {
    let mut bits_to_skip = 0;

    if is_usac == 0 {
        bits_to_skip = 6;
        fdk_push_for(h_bs, 6); // amp res 1, xover freq 3, reserved 2
    }
    bits_to_skip += 8;
    fdk_push_for(h_bs, 8); // start / stop freq
    bits_to_skip += 2;
    let dflt_header_extra1 = fdk_read_bit(h_bs) as i32;
    let dflt_header_extra2 = fdk_read_bit(h_bs) as i32;
    bits_to_skip += 5 * dflt_header_extra1 + 6 * dflt_header_extra2;
    fdk_push_for(h_bs, (5 * dflt_header_extra1 + 6 * dflt_header_extra2) as u32);

    bits_to_skip
}

/// `ld_sbr_header(asc, dsFactor, hBs, cb)`: calls the SBR header callback for every channel
/// element of the channel configuration.
fn ld_sbr_header(
    asc: &mut CSAudioSpecificConfig,
    ds_factor: i32,
    h_bs: &mut FdkBitStream,
    cb: &mut dyn TpDecCallbacks,
) -> i32 {
    let channel_configuration = asc.m_channel_configuration as i32;
    let mut i = 0;
    let mut j = 0;
    let mut error = 0;

    // check whether the channelConfiguration is defined in channel_configuration_array
    if channel_configuration < 0
        || channel_configuration > (CHANNEL_CONFIGURATION_ARRAY.len() - 1) as i32
    {
        return TRANSPORTDEC_PARSE_ERROR;
    }

    let list = CHANNEL_CONFIGURATION_ARRAY[channel_configuration as usize];
    // read elements of the passed channel_configuration until there is ID_NONE
    loop {
        let element = list[j];
        if element == ID_NONE {
            break;
        }
        // Setup LFE element for upsampling too. This is essential especially for channel configs
        // where the LFE element is not at the last position for example in channel config 13 or
        // 14. It leads to memory leaks if the setup of the LFE element would be done later in
        // the core.
        if element == ID_SCE || element == ID_CPE || element == ID_LFE {
            // UINT / INT: the division is unsigned (ds_factor is 1, 2 or 4).
            let ds = ds_factor as u32;
            error |= cb.sbr(
                h_bs,
                (asc.m_sampling_frequency / ds) as i32,
                (asc.m_extension_sampling_frequency / ds) as i32,
                (asc.m_samples_per_frame / ds) as i32,
                AOT_ER_AAC_ELD,
                element,
                i,
                0,
                0,
                asc.config_mode,
                &mut asc.sbr_config_changed,
                ds_factor,
            );
            i += 1;
            if error != TRANSPORTDEC_OK {
                break;
            }
        }
        j += 1;
    }
    error
}

/// `EldSpecificConfig_Parse(asc, hBs, cb)`
fn eld_specific_config_parse(
    asc: &mut CSAudioSpecificConfig,
    h_bs: &mut FdkBitStream,
    cb: &mut dyn TpDecCallbacks,
) -> TransportDecError {
    let mut error_status = TRANSPORTDEC_OK;
    let mut eld_ext_type: u32;
    let mut ld_sbr_len = 0i32;
    let mut num_sbr_header = 0;
    let mut eld_ext_cnt = 0;

    asc.m_sc.m_eld_specific_config = CSEldSpecificConfig::default();

    asc.m_sc.m_eld_specific_config.m_frame_length_flag = fdk_read_bits(h_bs, 1) as u8;
    if asc.m_sc.m_eld_specific_config.m_frame_length_flag != 0 {
        asc.m_samples_per_frame = 480;
    } else {
        asc.m_samples_per_frame = 512;
    }

    asc.m_vcb11_flag = fdk_read_bits(h_bs, 1) as i8;
    asc.m_rvlc_flag = fdk_read_bits(h_bs, 1) as i8;
    asc.m_hcr_flag = fdk_read_bits(h_bs, 1) as i8;

    asc.m_sc.m_eld_specific_config.m_sbr_present_flag = fdk_read_bits(h_bs, 1) as u8;

    if asc.m_sc.m_eld_specific_config.m_sbr_present_flag == 1 {
        // 0: single rate, 1: dual rate
        asc.m_sc.m_eld_specific_config.m_sbr_sampling_rate = fdk_read_bits(h_bs, 1) as u8;
        asc.m_sc.m_eld_specific_config.m_sbr_crc_flag = fdk_read_bits(h_bs, 1) as u8;

        asc.m_extension_sampling_frequency = asc
            .m_sampling_frequency
            .wrapping_shl(asc.m_sc.m_eld_specific_config.m_sbr_sampling_rate as u32);

        if cb.has_sbr() {
            // ELD reduced delay mode: LD-SBR initialization has to know the downscale
            // information. Postpone LD-SBR initialization and read ELD extension information
            // first.
            num_sbr_header = match asc.m_channel_configuration {
                1 | 2 => 1,
                3 => 2,
                4..=6 => 3,
                7 | 11 | 12 | 14 => 4,
                _ => 0,
            };
            for _ in 0..num_sbr_header {
                ld_sbr_len += skip_sbr_header(h_bs, 0);
            }
        } else {
            return TRANSPORTDEC_UNSUPPORTED_FORMAT;
        }
    }
    asc.m_sc.m_eld_specific_config.m_use_ld_qmf_time_align = 0;

    // new ELD syntax
    let mut eld_ext_len_sum = fdk_get_valid_bits(h_bs) as i32;
    asc.m_sc.m_eld_specific_config.m_downscaled_sampling_frequency = asc.m_sampling_frequency;
    // parse ExtTypeConfigData
    loop {
        eld_ext_type = fdk_read_bits(h_bs, 4);
        if eld_ext_type == ELDEXT_TERM {
            break;
        }
        if (fdk_get_valid_bits(h_bs) as i32) < 0 {
            break;
        }
        let cnt_ok = eld_ext_cnt < 15;
        eld_ext_cnt += 1;
        if !cnt_ok {
            break;
        }

        let mut len = fdk_read_bits(h_bs, 4) as i32;
        let mut eld_ext_len = len;
        if len == 0xf {
            len = fdk_read_bits(h_bs, 8) as i32;
            eld_ext_len += len;

            if len == 0xff {
                len = fdk_read_bits(h_bs, 16) as i32;
                eld_ext_len += len;
            }
        }

        let mut skip = false;
        match eld_ext_type {
            ELDEXT_LDSAC => {
                asc.m_sc.m_eld_specific_config.m_use_ld_qmf_time_align = 1;
                if cb.has_ssc() {
                    let sbr_sr = asc.m_sc.m_eld_specific_config.m_sbr_sampling_rate as u32;
                    error_status = cb.ssc(
                        h_bs,
                        asc.m_aot,
                        asc.m_sampling_frequency.wrapping_shl(sbr_sr) as i32,
                        asc.m_samples_per_frame.wrapping_shl(sbr_sr) as i32,
                        asc.m_channel_configuration as i32,
                        1,  // stereoConfigIndex
                        -1, // nTimeSlots: read from bitstream
                        eld_ext_len,
                        asc.config_mode,
                        &mut asc.sac_config_changed,
                    );
                    if error_status != TRANSPORTDEC_OK {
                        return TRANSPORTDEC_PARSE_ERROR;
                    }
                    if asc.m_sc.m_eld_specific_config.m_downscaled_sampling_frequency
                        != asc.m_sampling_frequency
                    {
                        // ELDv2 w/ ELD downscaled mode not allowed
                        return TRANSPORTDEC_UNSUPPORTED_FORMAT;
                    }
                } else {
                    skip = true; // FDK_FALLTHROUGH
                }
            }
            ELDEXT_DOWNSCALEINFO => {
                let mut tmp_downscale_freq_idx = 0u8;
                asc.m_sc.m_eld_specific_config.m_downscaled_sampling_frequency =
                    get_sample_rate(h_bs, &mut tmp_downscale_freq_idx, 4) as u32;
                if asc.m_sc.m_eld_specific_config.m_downscaled_sampling_frequency == 0
                    || asc.m_sc.m_eld_specific_config.m_downscaled_sampling_frequency > 96000
                {
                    return TRANSPORTDEC_PARSE_ERROR;
                }
                let downscale_fill_nibble = fdk_read_bits(h_bs, 4);
                if downscale_fill_nibble != 0x0 {
                    return TRANSPORTDEC_PARSE_ERROR;
                }
                if asc.m_sc.m_eld_specific_config.m_use_ld_qmf_time_align == 1 {
                    // ELDv2 w/ ELD downscaled mode not allowed
                    return TRANSPORTDEC_UNSUPPORTED_FORMAT;
                }
            }
            _ => skip = true,
        }
        if skip {
            for _ in 0..eld_ext_len {
                fdk_read_bits(h_bs, 8);
            }
        }
    }
    if eld_ext_type != ELDEXT_TERM {
        return TRANSPORTDEC_PARSE_ERROR;
    }

    if (fdk_get_valid_bits(h_bs) as i32) < 0 {
        return TRANSPORTDEC_PARSE_ERROR;
    }

    if asc.m_sc.m_eld_specific_config.m_sbr_present_flag == 1 && num_sbr_header != 0 {
        let mut ds_factor = 1i32; // Downscale factor must be 1 or even for SBR
        let dsf = asc.m_sc.m_eld_specific_config.m_downscaled_sampling_frequency;
        if dsf != 0 {
            if asc.m_sampling_frequency % dsf != 0 {
                return TRANSPORTDEC_UNSUPPORTED_FORMAT;
            }
            ds_factor = (asc.m_sampling_frequency / dsf) as i32;
            if ds_factor != 1 && (ds_factor & 1) != 0 {
                // SBR needs an even downscale factor
                return TRANSPORTDEC_UNSUPPORTED_FORMAT;
            }
            if ds_factor != 1 && ds_factor != 2 && ds_factor != 4 {
                ds_factor = 1; // don't apply dsf for not yet supported even dsfs
            }
            if (asc.m_samples_per_frame as i32) % ds_factor != 0 {
                // frameSize/dsf must be an integer number
                return TRANSPORTDEC_UNSUPPORTED_FORMAT;
            }
        }
        // INT - UINT: computed in unsigned arithmetic, stored as INT.
        eld_ext_len_sum = (eld_ext_len_sum as u32).wrapping_sub(fdk_get_valid_bits(h_bs)) as i32;
        fdk_push_back(h_bs, (eld_ext_len_sum as u32).wrapping_add(ld_sbr_len as u32));
        if 0 != ld_sbr_header(asc, ds_factor, h_bs, cb) {
            return TRANSPORTDEC_PARSE_ERROR;
        }
        fdk_push_for(h_bs, eld_ext_len_sum as u32);
    }
    error_status
}

/// `StoreConfigAsBitstream(hBs, configSize_bits, configTargetBuffer,
/// configTargetBufferSize_bytes)`: stores a configuration (before the current position for
/// `config_size_bits < 0`, after it for `> 0`) in a byte buffer. The bitstream position does not
/// change. Returns 1 if the configuration does not fit.
fn store_config_as_bitstream(
    h_bs: &mut FdkBitStream,
    config_size_bits: i32,
    config_target_buffer: &mut [u8; TP_USAC_MAX_CONFIG_LEN],
    config_target_buffer_size_bytes: u16,
) -> u32 {
    let mut usac_conf = FdkBitStream::default();
    // fAbs(INT): INT_MIN stays INT_MIN, which converts to 0x80000000.
    let n_bits = config_size_bits.wrapping_abs() as u32;

    if n_bits > 8 * config_target_buffer_size_bytes as u32 {
        return 1;
    }
    *config_target_buffer = [0; TP_USAC_MAX_CONFIG_LEN];

    // The writer works on a memory of the target buffer's size, copied back at the end.
    let mem = BitMem::new(TP_USAC_MAX_CONFIG_LEN);
    fdk_init_bit_stream(
        &mut usac_conf,
        &mem,
        config_target_buffer_size_bytes as u32,
        n_bits,
        BS_WRITER,
    );
    if config_size_bits < 0 {
        fdk_push_back(h_bs, n_bits);
    }
    let mut j = n_bits;
    while j > 31 {
        let tmp = fdk_read_bits(h_bs, 32);
        fdk_write_bits(&mut usac_conf, tmp, 32);
        j -= 32;
    }
    if j > 0 {
        let tmp = fdk_read_bits(h_bs, j);
        fdk_write_bits(&mut usac_conf, tmp, j);
    }
    fdk_sync_cache(&mut usac_conf);
    mem.read(0, config_target_buffer);
    if config_size_bits > 0 {
        fdk_push_back(h_bs, n_bits);
    }

    0
}

/// `usacFrameLength[]`: maps coreSbrFrameLengthIndex to coreCoderFrameLength.
///
/// `scripts/dump_table.py libMpegTPDec/src/tpdec_asc.cpp usacFrameLength USHORT`
pub static USAC_FRAME_LENGTH: [u16; 8] = [768, 1024, 2048, 2048, 4096, 0, 0, 0];
/// `sbrRatioIndex[]`: maps coreSbrFrameLengthIndex to sbrRatioIndex.
///
/// `scripts/dump_table.py libMpegTPDec/src/tpdec_asc.cpp sbrRatioIndex UCHAR`
pub static SBR_RATIO_INDEX: [u8; 8] = [0, 0, 2, 3, 1, 0, 0, 0];

/// `extElementConfig()`: `UsacExtElementConfig()` (ISO/IEC FDIS 23003-3:2011(E) Table 14).
#[allow(clippy::too_many_arguments)]
fn ext_element_config(
    ext_element: &mut CSUsacExtElementConfig,
    h_bs: &mut FdkBitStream,
    cb: &mut dyn TpDecCallbacks,
    _num_signals_in_group: u8,
    _core_frame_length: u32,
    sub_stream_index: i32,
    aot: AudioObjectType,
) -> TransportDecError {
    let mut error_status = TRANSPORTDEC_OK;

    let mut usac_ext_element_type = escaped_value(h_bs, 4, 8, 16);

    // recurve extension elements which are invalid for USAC
    if aot == AOT_USAC {
        match usac_ext_element_type as i32 {
            ID_EXT_ELE_FILL
            | ID_EXT_ELE_MPEGS
            | ID_EXT_ELE_SAOC
            | ID_EXT_ELE_AUDIOPREROLL
            | ID_EXT_ELE_UNI_DRC => {}
            _ => usac_ext_element_type = ID_EXT_ELE_UNKNOWN as u32,
        }
    }

    let usac_ext_element_config_length = escaped_value(h_bs, 4, 8, 16) as i32;
    ext_element.usac_ext_element_config_length = usac_ext_element_config_length as u16;

    if fdk_read_bit(h_bs) != 0 {
        // usacExtElementDefaultLengthPresent
        ext_element.usac_ext_element_default_length =
            escaped_value(h_bs, 8, 16, 0).wrapping_add(1) as u16;
    } else {
        ext_element.usac_ext_element_default_length = 0;
    }

    ext_element.usac_ext_element_payload_frag = fdk_read_bit(h_bs) as u8;

    let bs_anchor = fdk_get_valid_bits(h_bs) as i32;

    match usac_ext_element_type as i32 {
        ID_EXT_ELE_UNKNOWN | ID_EXT_ELE_FILL => {}
        ID_EXT_ELE_AUDIOPREROLL => {
            // No configuration element
            ext_element.usac_ext_element_has_audio_pre_roll = 1;
        }
        ID_EXT_ELE_UNI_DRC => {
            if cb.has_uni_drc() {
                error_status = cb.uni_drc(
                    Some(h_bs),
                    usac_ext_element_config_length,
                    0, // uniDrcConfig
                    sub_stream_index,
                    0,
                    aot,
                );
                if error_status != TRANSPORTDEC_OK {
                    return error_status;
                }
            }
        }
        _ => usac_ext_element_type = ID_EXT_ELE_UNKNOWN as u32,
    }
    ext_element.usac_ext_element_type = usac_ext_element_type as UsacExtElementType;

    // Adjust bit stream position. This is required because of byte alignment and unhandled
    // extensions.
    let left_bits = (usac_ext_element_config_length << 3)
        .wrapping_sub(bs_anchor.wrapping_sub(fdk_get_valid_bits(h_bs) as i32));
    if left_bits >= 0 {
        fdk_push_for(h_bs, left_bits as u32);
    } else {
        // parsed too many bits
        error_status = TRANSPORTDEC_PARSE_ERROR;
    }

    error_status
}

/// `configExtension()`: `UsacConfigExtension()` (ISO/IEC FDIS 23003-3:2011(E) Table 15).
fn config_extension(h_bs: &mut FdkBitStream, cb: &mut dyn TpDecCallbacks) -> TransportDecError {
    let mut error_status = TRANSPORTDEC_OK;

    let mut loudness_info_set_index = -1; // index of loudnessInfoSet config extension
    let tmp_sub_stream_index = 0;
    let tmp_aot = AOT_USAC;

    let num_config_extensions = escaped_value(h_bs, 2, 4, 8) as i32 + 1;
    for conf_ext_idx in 0..num_config_extensions {
        let loudness_info_set_config_extension_position = fdk_get_valid_bits(h_bs) as i32;
        let usac_config_ext_type = escaped_value(h_bs, 4, 8, 16);
        let mut usac_config_ext_length = escaped_value(h_bs, 4, 8, 16) as i32;

        // Start bit position of config extension
        let nbits = fdk_get_valid_bits(h_bs) as i32;

        // Return an error in case the bitbuffer fill level is too low.
        if nbits < usac_config_ext_length * 8 {
            return TRANSPORTDEC_PARSE_ERROR;
        }

        match usac_config_ext_type as i32 {
            ID_CONFIG_EXT_FILL => {
                for _ in 0..usac_config_ext_length {
                    if fdk_read_bits(h_bs, 8) != 0xa5 {
                        return TRANSPORTDEC_PARSE_ERROR;
                    }
                }
            }
            ID_CONFIG_EXT_LOUDNESS_INFO => {
                if cb.has_uni_drc() {
                    error_status = cb.uni_drc(
                        Some(h_bs),
                        usac_config_ext_length,
                        1, // loudnessInfoSet
                        tmp_sub_stream_index,
                        loudness_info_set_config_extension_position,
                        tmp_aot,
                    );
                    if error_status != TRANSPORTDEC_OK {
                        return error_status;
                    }
                    loudness_info_set_index = conf_ext_idx;
                }
            }
            _ => {}
        }

        // Skip remaining bits. If too many bits were parsed, assume error.
        usac_config_ext_length =
            8 * usac_config_ext_length - (nbits - fdk_get_valid_bits(h_bs) as i32);
        if usac_config_ext_length < 0 {
            return TRANSPORTDEC_PARSE_ERROR;
        }
        fdk_push_for(h_bs, usac_config_ext_length as u32);
    }

    if loudness_info_set_index == -1 && cb.has_uni_drc() {
        // no loudnessInfoSet contained. Clear the loudnessInfoSet struct by feeding an empty
        // config extension
        error_status =
            cb.uni_drc(None, 0, 1 /* loudnessInfoSet */, tmp_sub_stream_index, 0, tmp_aot);
        if error_status != TRANSPORTDEC_OK {
            return error_status;
        }
    }

    error_status
}

/// `UsacRsv60DecoderConfig_Parse(asc, hBs, cb)`: `UsacDecoderConfig()` (ISO/IEC FDIS 23003-3
/// Table 6).
fn usac_rsv60_decoder_config_parse(
    asc: &mut CSAudioSpecificConfig,
    h_bs: &mut FdkBitStream,
    cb: &mut dyn TpDecCallbacks,
) -> TransportDecError {
    let mut error_status = TRANSPORTDEC_OK;
    // index for elements which contain audio channels (sce, cpe, lfe)
    let mut channel_element_idx = 0i32;
    let mut sc_chan_config = ScChannelConfig::default();
    let mut uni_drc_element = -1i32; // index of uniDrc extension element

    let number_of_elements = escaped_value(h_bs, 4, 8, 16) as i32 + 1;
    asc.m_sc.m_usac_config.m_usac_num_elements = number_of_elements as u32;
    if number_of_elements > TP_USAC_MAX_ELEMENTS as i32 {
        return TRANSPORTDEC_UNSUPPORTED_FORMAT;
    }
    asc.m_sc.m_usac_config.m_n_usac_channels = 0;
    asc.m_sc.m_usac_config.m_channel_configuration_index = asc.m_channel_configuration as u8;

    if asc.m_aot == AOT_USAC {
        // The index is 1 or 2 (checked by the caller); the table has 14 entries.
        sc_chan_config = SC_CHAN_CONFIG_TAB
            .get(asc.m_sc.m_usac_config.m_channel_configuration_index as usize)
            .copied()
            .unwrap_or_default();

        if sc_chan_config.n_ch > TP_USAC_MAX_SPEAKERS as i8 {
            return TRANSPORTDEC_PARSE_ERROR;
        }
    }

    for i in 0..number_of_elements as usize {
        // set USAC_ID_BIT to map usacElementType to MP4_ELEMENT_ID enum
        let usac_element_type = (fdk_read_bits(h_bs, 2) as i32) | USAC_ID_BIT;
        asc.m_sc.m_usac_config.element[i].usac_element_type = usac_element_type;

        // sanity check: update element counter
        if asc.m_aot == AOT_USAC {
            match usac_element_type {
                ID_USAC_SCE => sc_chan_config.n_sce = sc_chan_config.n_sce.wrapping_sub(1),
                ID_USAC_CPE => sc_chan_config.n_cpe = sc_chan_config.n_cpe.wrapping_sub(1),
                ID_USAC_LFE => sc_chan_config.n_lfe = sc_chan_config.n_lfe.wrapping_sub(1),
                _ => {}
            }
            if asc.m_sc.m_usac_config.m_channel_configuration_index != 0 {
                // sanity check: no element counter may be smaller zero
                if sc_chan_config.n_cpe < 0 || sc_chan_config.n_sce < 0 || sc_chan_config.n_lfe < 0
                {
                    return TRANSPORTDEC_PARSE_ERROR;
                }
            }
        }

        match usac_element_type {
            ID_USAC_SCE => {
                // UsacCoreConfig() ISO/IEC FDIS 23003-3  Table 10
                if fdk_read_bit(h_bs) != 0 {
                    // tw_mdct
                    return TRANSPORTDEC_UNSUPPORTED_FORMAT;
                }
                asc.m_sc.m_usac_config.element[i].m_noise_filling = fdk_read_bits(h_bs, 1) as u8;
                // end of UsacCoreConfig()
                if asc.m_sc.m_usac_config.m_sbr_ratio_index > 0 {
                    if !cb.has_sbr() {
                        return TRANSPORTDEC_UNKOWN_ERROR;
                    }
                    // SbrConfig() ISO/IEC FDIS 23003-3  Table 11
                    let e = &mut asc.m_sc.m_usac_config.element[i];
                    e.m_harmonic_sbr = fdk_read_bit(h_bs) as u8;
                    e.m_inter_tes = fdk_read_bit(h_bs) as u8;
                    e.m_pvc = fdk_read_bit(h_bs) as u8;
                    let (harmonic_sbr, stereo_config_index) =
                        (e.m_harmonic_sbr, e.m_stereo_config_index);
                    if cb.sbr(
                        h_bs,
                        asc.m_sampling_frequency as i32,
                        asc.m_extension_sampling_frequency as i32,
                        asc.m_samples_per_frame as i32,
                        asc.m_aot,
                        ID_SCE,
                        channel_element_idx,
                        harmonic_sbr,
                        stereo_config_index,
                        asc.config_mode,
                        &mut asc.sbr_config_changed,
                        1,
                    ) != 0
                    {
                        return TRANSPORTDEC_PARSE_ERROR;
                    }
                    // end of SbrConfig()
                }
                asc.m_sc.m_usac_config.m_n_usac_channels =
                    asc.m_sc.m_usac_config.m_n_usac_channels.wrapping_add(1);
                channel_element_idx += 1;
            }

            ID_USAC_CPE => {
                // UsacCoreConfig() ISO/IEC FDIS 23003-3  Table 10
                if fdk_read_bit(h_bs) != 0 {
                    // tw_mdct
                    return TRANSPORTDEC_UNSUPPORTED_FORMAT;
                }
                asc.m_sc.m_usac_config.element[i].m_noise_filling = fdk_read_bits(h_bs, 1) as u8;
                // end of UsacCoreConfig()
                if asc.m_sc.m_usac_config.m_sbr_ratio_index > 0 {
                    if !cb.has_sbr() {
                        return TRANSPORTDEC_UNKOWN_ERROR;
                    }
                    // SbrConfig() ISO/IEC FDIS 23003-3
                    {
                        let e = &mut asc.m_sc.m_usac_config.element[i];
                        e.m_harmonic_sbr = fdk_read_bit(h_bs) as u8;
                        e.m_inter_tes = fdk_read_bit(h_bs) as u8;
                        e.m_pvc = fdk_read_bit(h_bs) as u8;
                        let bits_to_skip = skip_sbr_header(h_bs, 1);
                        // read stereoConfigIndex
                        e.m_stereo_config_index = fdk_read_bits(h_bs, 2) as u8;
                        // rewind
                        fdk_push_back(h_bs, (bits_to_skip + 2) as u32);
                    }
                    {
                        let e = asc.m_sc.m_usac_config.element[i];
                        let el_type =
                            if e.m_stereo_config_index == 1 || e.m_stereo_config_index == 2 {
                                ID_SCE
                            } else {
                                ID_CPE
                            };
                        if cb.sbr(
                            h_bs,
                            asc.m_sampling_frequency as i32,
                            asc.m_extension_sampling_frequency as i32,
                            asc.m_samples_per_frame as i32,
                            asc.m_aot,
                            el_type,
                            channel_element_idx,
                            e.m_harmonic_sbr,
                            e.m_stereo_config_index,
                            asc.config_mode,
                            &mut asc.sbr_config_changed,
                            1,
                        ) != 0
                        {
                            return TRANSPORTDEC_PARSE_ERROR;
                        }
                    }
                    // end of SbrConfig()

                    // Needed in RM5 syntax
                    asc.m_sc.m_usac_config.element[i].m_stereo_config_index =
                        fdk_read_bits(h_bs, 2) as u8;

                    if asc.m_sc.m_usac_config.element[i].m_stereo_config_index > 0 {
                        if cb.has_ssc() {
                            let mut samples_per_frame = asc.m_samples_per_frame as i32;
                            let ratio = asc.m_sc.m_usac_config.m_sbr_ratio_index;

                            if ratio == 1 {
                                samples_per_frame <<= 2;
                            }
                            if ratio == 2 {
                                samples_per_frame = (samples_per_frame * 8) / 3;
                            }
                            if ratio == 3 {
                                samples_per_frame <<= 1;
                            }

                            // Mps212Config() ISO/IEC FDIS 23003-3
                            if cb.ssc(
                                h_bs,
                                asc.m_aot,
                                asc.m_extension_sampling_frequency as i32,
                                samples_per_frame,
                                1, // only downmix channels (residual channels are not counted)
                                asc.m_sc.m_usac_config.element[i].m_stereo_config_index as i32,
                                asc.m_sc.m_usac_config.m_core_sbr_frame_length_index as i32,
                                0, // don't know the length
                                asc.config_mode,
                                &mut asc.sac_config_changed,
                            ) != 0
                            {
                                return TRANSPORTDEC_PARSE_ERROR;
                            }
                            // end of Mps212Config()
                        } else {
                            return TRANSPORTDEC_UNKOWN_ERROR;
                        }
                    }
                } else {
                    asc.m_sc.m_usac_config.element[i].m_stereo_config_index = 0;
                }
                asc.m_sc.m_usac_config.m_n_usac_channels =
                    asc.m_sc.m_usac_config.m_n_usac_channels.wrapping_add(2);

                channel_element_idx += 1;
            }

            ID_USAC_LFE => {
                asc.m_sc.m_usac_config.element[i].m_noise_filling = 0;
                asc.m_sc.m_usac_config.m_n_usac_channels =
                    asc.m_sc.m_usac_config.m_n_usac_channels.wrapping_add(1);
                if asc.m_sc.m_usac_config.m_sbr_ratio_index > 0 {
                    // Use SBR for upsampling
                    if !cb.has_sbr() {
                        return TRANSPORTDEC_UNKOWN_ERROR;
                    }
                    let e = &mut asc.m_sc.m_usac_config.element[i];
                    e.m_harmonic_sbr = 0;
                    e.m_inter_tes = 0;
                    e.m_pvc = 0;
                    let (harmonic_sbr, stereo_config_index) =
                        (e.m_harmonic_sbr, e.m_stereo_config_index);
                    if cb.sbr(
                        h_bs,
                        asc.m_sampling_frequency as i32,
                        asc.m_extension_sampling_frequency as i32,
                        asc.m_samples_per_frame as i32,
                        asc.m_aot,
                        ID_LFE,
                        channel_element_idx,
                        harmonic_sbr,
                        stereo_config_index,
                        asc.config_mode,
                        &mut asc.sbr_config_changed,
                        1,
                    ) != 0
                    {
                        return TRANSPORTDEC_PARSE_ERROR;
                    }
                }
                channel_element_idx += 1;
            }

            ID_USAC_EXT => {
                let samples_per_frame = asc.m_samples_per_frame;
                let aot = asc.m_aot;
                error_status = ext_element_config(
                    &mut asc.m_sc.m_usac_config.element[i].ext_element,
                    h_bs,
                    cb,
                    0,
                    samples_per_frame,
                    0,
                    aot,
                );
                if asc.m_sc.m_usac_config.element[i].ext_element.usac_ext_element_type
                    == ID_EXT_ELE_UNI_DRC
                {
                    uni_drc_element = i as i32;
                }

                if error_status != 0 {
                    return error_status;
                }
            }

            _ => {
                // non USAC-element encountered
                return TRANSPORTDEC_PARSE_ERROR;
            }
        }
    }

    if asc.m_aot == AOT_USAC {
        if asc.m_sc.m_usac_config.m_channel_configuration_index != 0 {
            // sanity check: all element counter must be zero
            if (sc_chan_config.n_cpe | sc_chan_config.n_sce | sc_chan_config.n_lfe) != 0 {
                return TRANSPORTDEC_PARSE_ERROR;
            }
        } else {
            // sanity check: number of audio channels shall be equal to or smaller than the
            // accumulated sum of all channels
            if (-2 * sc_chan_config.n_cpe as i32
                - sc_chan_config.n_sce as i32
                - sc_chan_config.n_lfe as i32)
                < asc.m_sc.m_usac_config.num_audio_channels as i32
            {
                return TRANSPORTDEC_PARSE_ERROR;
            }
        }
    }

    if uni_drc_element == -1 && cb.has_uni_drc() {
        // no uniDrcConfig contained. Clear the uniDrcConfig struct by feeding an empty extension
        // element
        let sub_stream_index = 0;
        error_status =
            cb.uni_drc(None, 0, 0 /* uniDrcConfig */, sub_stream_index, 0, asc.m_aot);
        if error_status != TRANSPORTDEC_OK {
            return error_status;
        }
    }

    error_status
}

/// `UsacConfig_SetCoreSbrFrameLengthIndex(asc, coreSbrFrameLengthIndex)`: mapping of
/// coreSbrFrameLengthIndex defined by Table 70 in ISO/IEC 23003-3.
fn usac_config_set_core_sbr_frame_length_index(
    asc: &mut CSAudioSpecificConfig,
    core_sbr_frame_length_index: i32,
) -> TransportDecError {
    if core_sbr_frame_length_index > 4 {
        return TRANSPORTDEC_PARSE_ERROR; // reserved values
    }
    // The index is never negative (read from 3 or 2 bits).
    let idx = core_sbr_frame_length_index as usize & 7;
    asc.m_sc.m_usac_config.m_core_sbr_frame_length_index = core_sbr_frame_length_index as u8;
    asc.m_samples_per_frame = USAC_FRAME_LENGTH[idx] as u32;
    let sbr_ratio_index_val = SBR_RATIO_INDEX[idx] as i32;
    asc.m_sc.m_usac_config.m_sbr_ratio_index = sbr_ratio_index_val as u8;

    if sbr_ratio_index_val > 0 {
        asc.m_sbr_present_flag = 1;
        asc.m_extension_sampling_frequency = asc.m_sampling_frequency;
        asc.m_extension_sampling_frequency_index = asc.m_sampling_frequency_index;
        match sbr_ratio_index_val {
            1 => {
                // sbrRatio = 4:1
                asc.m_sampling_frequency >>= 2;
                asc.m_samples_per_frame >>= 2;
            }
            2 => {
                // sbrRatio = 8:3
                asc.m_sampling_frequency = asc.m_sampling_frequency.wrapping_mul(3) / 8;
                asc.m_samples_per_frame = asc.m_samples_per_frame.wrapping_mul(3) / 8;
            }
            3 => {
                // sbrRatio = 2:1
                asc.m_sampling_frequency >>= 1;
                asc.m_samples_per_frame >>= 1;
            }
            _ => return TRANSPORTDEC_PARSE_ERROR,
        }
        asc.m_sampling_frequency_index = get_sampling_rate_index(asc.m_sampling_frequency, 4) as u8;
    }

    TRANSPORTDEC_OK
}

/// `UsacConfig_Parse(asc, hBs, cb)`
fn usac_config_parse(
    asc: &mut CSAudioSpecificConfig,
    h_bs: &mut FdkBitStream,
    cb: &mut dyn TpDecCallbacks,
) -> TransportDecError {
    // Start bit position of usacConfig
    let nbits = fdk_get_valid_bits(h_bs) as i32;

    let usac_sampling_frequency = get_sample_rate(h_bs, &mut asc.m_sampling_frequency_index, 5);
    if usac_sampling_frequency == 0 || usac_sampling_frequency > 96000 {
        return TRANSPORTDEC_PARSE_ERROR;
    }
    asc.m_sampling_frequency = usac_sampling_frequency as u32;

    let core_sbr_frame_length_index = fdk_read_bits(h_bs, 3) as i32;
    if usac_config_set_core_sbr_frame_length_index(asc, core_sbr_frame_length_index)
        != TRANSPORTDEC_OK
    {
        return TRANSPORTDEC_PARSE_ERROR;
    }

    let channel_configuration_index = fdk_read_bits(h_bs, 5) as i32;
    if channel_configuration_index > 2 {
        // only channelConfigurationIndex = [1,2] are supported
        return TRANSPORTDEC_PARSE_ERROR;
    }

    if channel_configuration_index == 0 {
        // only channelConfigurationIndex = [1,2] are supported
        return TRANSPORTDEC_PARSE_ERROR;
    }
    asc.m_channel_configuration = channel_configuration_index as i8;

    let mut err = usac_rsv60_decoder_config_parse(asc, h_bs, cb);
    if err != TRANSPORTDEC_OK {
        return err;
    }

    if fdk_read_bits(h_bs, 1) != 0 {
        // usacConfigExtensionPresent
        err = config_extension(h_bs, cb);
        if err != TRANSPORTDEC_OK {
            return err;
        }
    } else if cb.has_uni_drc() {
        // no loudnessInfoSet contained. Clear the loudnessInfoSet struct by feeding an empty
        // config extension
        err = cb.uni_drc(None, 0, 1 /* loudnessInfoSet */, 0, 0, asc.m_aot);
        if err != TRANSPORTDEC_OK {
            return err;
        }
    }

    // sanity check whether number of channels signaled in UsacDecoderConfig() matches the
    // number of channels required by channelConfigurationIndex
    if channel_configuration_index > 0
        && SC_CHAN_CONFIG_TAB[channel_configuration_index as usize].n_ch as i32
            != asc.m_sc.m_usac_config.m_n_usac_channels as i32
    {
        return TRANSPORTDEC_PARSE_ERROR;
    }

    // Copy UsacConfig() to asc->m_sc.m_usacConfig.UsacConfig[] buffer.
    let config_size_bits = (fdk_get_valid_bits(h_bs) as i32).wrapping_sub(nbits);
    if store_config_as_bitstream(
        h_bs,
        config_size_bits,
        &mut asc.m_sc.m_usac_config.usac_config,
        TP_USAC_MAX_CONFIG_LEN as u16,
    ) != 0
    {
        return TRANSPORTDEC_PARSE_ERROR;
    }
    asc.m_sc.m_usac_config.usac_config_bits = config_size_bits.wrapping_abs() as u16;

    err
}

/// `AudioSpecificConfig_ExtensionParse(self, bs, cb)`: the backward compatible explicit
/// signaling of SBR, PS and MPEG Surround.
fn audio_specific_config_extension_parse(
    self_: &mut CSAudioSpecificConfig,
    bs: &mut FdkBitStream,
) -> TransportDecError {
    let mut asc_ext_id: TpAscExtensionId = ASCEXT_UNKOWN;
    let mut bits_available = fdk_get_valid_bits(bs) as i32;

    while bits_available >= 11 {
        let last_asc_ext = asc_ext_id;
        asc_ext_id = fdk_read_bits(bs, 11) as TpAscExtensionId;
        bits_available -= 11;

        match asc_ext_id {
            ASCEXT_SBR => {
                // 0x2b7
                if self_.m_extension_audio_object_type != AOT_SBR && bits_available >= 5 {
                    self_.m_extension_audio_object_type = get_aot(bs);

                    if self_.m_extension_audio_object_type == AOT_SBR
                        || self_.m_extension_audio_object_type == AOT_ER_BSAC
                    {
                        // Get SBR extension configuration
                        self_.m_sbr_present_flag = fdk_read_bits(bs, 1) as i8;
                        if self_.m_aot == AOT_USAC
                            && self_.m_sbr_present_flag > 0
                            && self_.m_sc.m_usac_config.m_sbr_ratio_index == 0
                        {
                            return TRANSPORTDEC_PARSE_ERROR;
                        }

                        if self_.m_sbr_present_flag == 1 {
                            self_.m_extension_sampling_frequency = get_sample_rate(
                                bs,
                                &mut self_.m_extension_sampling_frequency_index,
                                4,
                            )
                                as u32;

                            if self_.m_extension_sampling_frequency == 0
                                || self_.m_extension_sampling_frequency > 96000
                            {
                                return TRANSPORTDEC_PARSE_ERROR;
                            }
                        }
                        if self_.m_extension_audio_object_type == AOT_ER_BSAC {
                            self_.m_extension_channel_configuration = fdk_read_bits(bs, 4) as i8;
                        }
                    }
                    // Update counter because of variable length fields (AOT and sampling rate)
                    bits_available = fdk_get_valid_bits(bs) as i32;
                }
            }
            ASCEXT_PS => {
                // 0x548
                if last_asc_ext == ASCEXT_SBR
                    && self_.m_extension_audio_object_type == AOT_SBR
                    && bits_available > 0
                {
                    // Get PS extension configuration
                    self_.m_ps_present_flag = fdk_read_bits(bs, 1) as i8;
                    bits_available -= 1;
                }
            }
            ASCEXT_MPS | ASCEXT_LDMPS => {
                // 0x76a, 0x7cc
                let skip = if asc_ext_id == ASCEXT_MPS {
                    self_.m_extension_audio_object_type == AOT_MPEGS
                } else {
                    self_.m_extension_audio_object_type == AOT_LD_MPEGS
                };
                if !skip && bits_available >= 1 {
                    bits_available -= 1;
                    if fdk_read_bits(bs, 1) != 0 {
                        // self->m_mpsPresentFlag
                        let mut ssc_len = fdk_read_bits(bs, 8) as i32;
                        bits_available -= 8;
                        if ssc_len == 0xFF {
                            ssc_len += fdk_read_bits(bs, 16) as i32;
                            bits_available -= 16;
                        }
                        // Skip SSC to be able to read the next extension if there is one.
                        // (Skips sscLen bits, but counts sscLen bytes.)
                        fdk_push_for(bs, ssc_len as u32);

                        bits_available = bits_available.wrapping_sub(ssc_len * 8);
                    }
                }
            }
            ASCEXT_SAOC => {
                if self_.m_extension_audio_object_type != AOT_SAOC && fdk_read_bits(bs, 1) != 0 {
                    // saocPresent
                    let mut saocsc_len = fdk_read_bits(bs, 8) as i32;
                    bits_available -= 8;
                    if saocsc_len == 0xFF {
                        saocsc_len += fdk_read_bits(bs, 16) as i32;
                        bits_available -= 16;
                    }
                    fdk_push_for(bs, saocsc_len as u32);
                    bits_available = bits_available.wrapping_sub(saocsc_len * 8);
                }
            }
            _ => {
                // Just ignore anything.
                return TRANSPORTDEC_OK;
            }
        }
    }

    TRANSPORTDEC_OK
}

// ---------------------------------------------------------------------------------------------
// API functions
// ---------------------------------------------------------------------------------------------

/// `AudioSpecificConfig_Init(asc)`
pub fn audio_specific_config_init(asc: &mut CSAudioSpecificConfig) {
    *asc = CSAudioSpecificConfig::zeroed();

    // Init all values that should not be zero.
    asc.m_aot = AOT_NONE;
    asc.m_sampling_frequency_index = 0xf;
    asc.m_ep_config = -1;
    asc.m_extension_audio_object_type = AOT_NULL_OBJECT;
    cprogram_config_init(&mut asc.m_progr_config_element);
}

/// `AudioSpecificConfig_Parse(self, bs, fExplicitBackwardCompatible, cb, configMode,
/// configChanged, m_aot)`: parses an AudioSpecificConfig (or, with `m_aot != AOT_NULL_OBJECT`,
/// only the specific config of that audio object type).
pub fn audio_specific_config_parse(
    self_: &mut CSAudioSpecificConfig,
    bs: &mut FdkBitStream,
    f_explicit_backward_compatible: i32,
    cb: &mut dyn TpDecCallbacks,
    config_mode: u8,
    config_changed: u8,
    m_aot: AudioObjectType,
) -> TransportDecError {
    let mut error_status = TRANSPORTDEC_OK;
    let asc_start_anchor = fdk_get_valid_bits(bs);
    let mut frame_length_flag: i32 = -1;

    audio_specific_config_init(self_);

    self_.config_mode = config_mode;
    self_.aac_config_changed = config_changed;
    self_.sbr_config_changed = config_changed;
    self_.sac_config_changed = config_changed;

    if m_aot != AOT_NULL_OBJECT {
        self_.m_aot = m_aot;
    } else {
        self_.m_aot = get_aot(bs);
        self_.m_sampling_frequency =
            get_sample_rate(bs, &mut self_.m_sampling_frequency_index, 4) as u32;
        if self_.m_sampling_frequency == 0
            || (self_.m_sampling_frequency > 96000 && self_.m_aot != 39)
            || self_.m_sampling_frequency > 4 * 96000
        {
            return TRANSPORTDEC_PARSE_ERROR;
        }

        self_.m_channel_configuration = fdk_read_bits(bs, 4) as i8;

        // MPEG-04 standard ISO/IEC 14496-3: channelConfiguration == 0 is reserved in
        // er_raw_data_block (table 4.19) and er_raw_data_block_eld (table 4.75)
        // MPEG-04 conformance ISO/IEC 14496-4: channelConfiguration == 0 is not permitted for
        // AOT_ER_AAC_LC, AOT_ER_AAC_LTP, AOT_ER_AAC_LD, AOT_ER_AAC_SCAL (chapter 6.6.4.1.2.1.1)
        if self_.m_channel_configuration == 0
            && (self_.m_aot == AOT_ER_AAC_LC
                || self_.m_aot == AOT_ER_AAC_LTP
                || self_.m_aot == AOT_ER_AAC_LD
                || self_.m_aot == AOT_ER_AAC_SCAL
                || self_.m_aot == AOT_ER_AAC_ELD)
        {
            return TRANSPORTDEC_UNSUPPORTED_FORMAT;
        }
        // MPEG-04 conformance ISO/IEC 14496-4: channelConfiguration > 2 is not permitted for
        // AOT_AAC_SCAL and AOT_ER_AAC_SCAL (chapter 6.6.4.1.2.1.1)
        if self_.m_channel_configuration > 2
            && (self_.m_aot == AOT_AAC_SCAL || self_.m_aot == AOT_ER_AAC_SCAL)
        {
            return TRANSPORTDEC_UNSUPPORTED_FORMAT;
        }

        // SBR extension ( explicit non-backwards compatible mode )
        self_.m_sbr_present_flag = 0;
        self_.m_ps_present_flag = 0;

        if self_.m_aot == AOT_SBR || self_.m_aot == AOT_PS {
            self_.m_extension_audio_object_type = AOT_SBR;

            self_.m_sbr_present_flag = 1;
            if self_.m_aot == AOT_PS {
                self_.m_ps_present_flag = 1;
            }

            self_.m_extension_sampling_frequency =
                get_sample_rate(bs, &mut self_.m_extension_sampling_frequency_index, 4) as u32;
            if self_.m_extension_sampling_frequency == 0
                || self_.m_extension_sampling_frequency > 96000
            {
                return TRANSPORTDEC_PARSE_ERROR;
            }
            self_.m_aot = get_aot(bs);

            match self_.m_aot {
                AOT_AAC_LC | AOT_ER_BSAC => {}
                _ => return TRANSPORTDEC_UNSUPPORTED_FORMAT,
            }

            if self_.m_aot == AOT_ER_BSAC {
                self_.m_extension_channel_configuration = fdk_read_bits(bs, 4) as i8;
            }
        } else {
            self_.m_extension_audio_object_type = AOT_NULL_OBJECT;
        }
    }

    // Parse whatever specific configs
    match self_.m_aot {
        AOT_AAC_LC | AOT_AAC_SCAL | AOT_ER_AAC_LC | AOT_ER_AAC_LD | AOT_ER_AAC_SCAL
        | AOT_ER_BSAC => {
            error_status = ga_specific_config_parse(self_, bs, asc_start_anchor);
            if error_status != TRANSPORTDEC_OK {
                return error_status;
            }
            frame_length_flag = self_.m_sc.m_ga_specific_config.m_frame_length_flag as i32;
        }
        AOT_MPEGS => {
            if cb.has_ssc() {
                if cb.ssc(
                    bs,
                    self_.m_aot,
                    self_.m_sampling_frequency as i32,
                    self_.m_samples_per_frame as i32,
                    self_.m_channel_configuration as i32,
                    1,
                    -1, // nTimeSlots: read from bitstream
                    0,  // don't know the length
                    self_.config_mode,
                    &mut self_.sac_config_changed,
                ) != 0
                {
                    return TRANSPORTDEC_UNSUPPORTED_FORMAT;
                }
            } else {
                return TRANSPORTDEC_UNSUPPORTED_FORMAT;
            }
        }
        AOT_ER_AAC_ELD => {
            error_status = eld_specific_config_parse(self_, bs, cb);
            if error_status != TRANSPORTDEC_OK {
                return error_status;
            }
            frame_length_flag = self_.m_sc.m_eld_specific_config.m_frame_length_flag as i32;
            self_.m_sbr_present_flag = self_.m_sc.m_eld_specific_config.m_sbr_present_flag as i8;
            self_.m_extension_sampling_frequency =
                ((self_.m_sc.m_eld_specific_config.m_sbr_sampling_rate as u32) + 1)
                    .wrapping_mul(self_.m_sampling_frequency);
        }
        AOT_USAC => {
            error_status = usac_config_parse(self_, bs, cb);
            if error_status != TRANSPORTDEC_OK {
                return error_status;
            }
        }

        _ => return TRANSPORTDEC_UNSUPPORTED_FORMAT,
    }

    // Frame length
    match self_.m_aot {
        AOT_AAC_LC | AOT_AAC_SCAL | AOT_ER_AAC_LC | AOT_ER_AAC_SCAL | AOT_ER_BSAC => {
            // case AOT_USAC:
            if frame_length_flag == 0 {
                self_.m_samples_per_frame = 1024;
            } else {
                self_.m_samples_per_frame = 960;
            }
        }
        AOT_ER_AAC_LD => {
            if frame_length_flag == 0 {
                self_.m_samples_per_frame = 512;
            } else {
                self_.m_samples_per_frame = 480;
            }
        }
        _ => {}
    }

    match self_.m_aot {
        AOT_ER_AAC_LC | AOT_ER_AAC_LD | AOT_ER_AAC_ELD | AOT_ER_AAC_SCAL | AOT_ER_CELP
        | AOT_ER_HVXC | AOT_ER_BSAC => {
            self_.m_ep_config = fdk_read_bits(bs, 2) as i8;

            if self_.m_ep_config > 1 {
                return TRANSPORTDEC_UNSUPPORTED_FORMAT; // EPCONFIG;
            }
        }
        _ => {}
    }

    if f_explicit_backward_compatible != 0
        && (self_.m_aot == AOT_AAC_LC || self_.m_aot == AOT_ER_AAC_LD || self_.m_aot == AOT_ER_BSAC)
    {
        error_status = audio_specific_config_extension_parse(self_, bs);
    }

    // Copy config() to asc->config[] buffer.
    if error_status == TRANSPORTDEC_OK && self_.m_aot == AOT_USAC {
        let config_size_bits =
            (fdk_get_valid_bits(bs) as i32).wrapping_sub(asc_start_anchor as i32);
        if store_config_as_bitstream(
            bs,
            config_size_bits,
            &mut self_.config,
            TP_USAC_MAX_CONFIG_LEN as u16,
        ) != 0
        {
            return TRANSPORTDEC_PARSE_ERROR;
        }
        self_.config_bits = config_size_bits.wrapping_abs() as u32;
    }

    error_status
}

/// `Drm_xHEAACDecoderConfig(asc, hBs, audioMode, cb)`: `cb == None` signals the config check
/// only mode.
fn drm_xhe_aac_decoder_config(
    asc: &mut CSAudioSpecificConfig,
    h_bs: &mut FdkBitStream,
    audio_mode: i32,
    cb: Option<&mut dyn TpDecCallbacks>,
) -> TransportDecError {
    let mut error_status = TRANSPORTDEC_OK;
    let elem_idx = 0usize;

    asc.m_sc.m_usac_config.element[elem_idx].m_stereo_config_index = 0;

    // Currently all extension elements are skipped -> only one SCE or CPE.
    asc.m_sc.m_usac_config.m_usac_num_elements = 1;

    match audio_mode {
        0 => {
            // mono: ID_USAC_SCE
            asc.m_sc.m_usac_config.element[elem_idx].usac_element_type = ID_USAC_SCE;
            asc.m_sc.m_usac_config.m_n_usac_channels = 1;
            asc.m_sc.m_usac_config.element[elem_idx].m_noise_filling = fdk_read_bits(h_bs, 1) as u8;
            if asc.m_sc.m_usac_config.m_sbr_ratio_index > 0 {
                let Some(cb) = cb else {
                    return error_status;
                };
                if cb.has_sbr() {
                    let e = &mut asc.m_sc.m_usac_config.element[elem_idx];
                    e.m_harmonic_sbr = fdk_read_bit(h_bs) as u8;
                    e.m_inter_tes = fdk_read_bit(h_bs) as u8;
                    e.m_pvc = fdk_read_bit(h_bs) as u8;
                    let (harmonic_sbr, stereo_config_index) =
                        (e.m_harmonic_sbr, e.m_stereo_config_index);
                    if cb.sbr(
                        h_bs,
                        asc.m_sampling_frequency as i32,
                        asc.m_extension_sampling_frequency as i32,
                        asc.m_samples_per_frame as i32,
                        asc.m_aot,
                        ID_SCE,
                        elem_idx as i32,
                        harmonic_sbr,
                        stereo_config_index,
                        asc.config_mode,
                        &mut asc.sbr_config_changed,
                        1,
                    ) != 0
                    {
                        return TRANSPORTDEC_PARSE_ERROR;
                    }
                }
            }
        }
        2 => {
            // stereo: ID_USAC_CPE
            asc.m_sc.m_usac_config.element[elem_idx].usac_element_type = ID_USAC_CPE;
            asc.m_sc.m_usac_config.m_n_usac_channels = 2;
            asc.m_sc.m_usac_config.element[elem_idx].m_noise_filling = fdk_read_bits(h_bs, 1) as u8;
            if asc.m_sc.m_usac_config.m_sbr_ratio_index > 0 {
                {
                    let e = &mut asc.m_sc.m_usac_config.element[elem_idx];
                    e.m_harmonic_sbr = fdk_read_bit(h_bs) as u8;
                    e.m_inter_tes = fdk_read_bit(h_bs) as u8;
                    e.m_pvc = fdk_read_bit(h_bs) as u8;
                    let bits_to_skip = skip_sbr_header(h_bs, 1);
                    // read stereoConfigIndex
                    e.m_stereo_config_index = fdk_read_bits(h_bs, 2) as u8;
                    // rewind
                    fdk_push_back(h_bs, (bits_to_skip + 2) as u32);
                }
                let e = asc.m_sc.m_usac_config.element[elem_idx];
                // The application of the following tools is mutually exclusive per audio stream
                // configuration (see clause 5.3.2, xHE-AAC codec configuration):
                // - MPS212 parametric stereo tool with residual coding (stereoConfigIndex>1);
                //   and
                // - QMF based Harmonic Transposer (harmonicSBR==1).
                if e.m_stereo_config_index > 1 && e.m_harmonic_sbr != 0 {
                    return TRANSPORTDEC_PARSE_ERROR;
                }
                // The 4:1 sbrRatio (sbrRatioIndex==1 in [11]) may only be employed:
                // - in mono operation; or
                // - in stereo operation if parametric stereo (MPS212) without residual coding is
                //   applied, i.e. if stereoConfigIndex==1 (see clause 5.3.2, xHE-AAC codec
                //   configuration).
                if asc.m_sc.m_usac_config.m_sbr_ratio_index == 1 && e.m_stereo_config_index != 1 {
                    return TRANSPORTDEC_PARSE_ERROR;
                }
                let Some(cb) = cb else {
                    return error_status;
                };
                {
                    let el_type = if e.m_stereo_config_index == 1 || e.m_stereo_config_index == 2 {
                        ID_SCE
                    } else {
                        ID_CPE
                    };
                    if !cb.has_sbr() {
                        return TRANSPORTDEC_UNKOWN_ERROR;
                    }
                    if cb.sbr(
                        h_bs,
                        asc.m_sampling_frequency as i32,
                        asc.m_extension_sampling_frequency as i32,
                        asc.m_samples_per_frame as i32,
                        asc.m_aot,
                        el_type,
                        elem_idx as i32,
                        e.m_harmonic_sbr,
                        e.m_stereo_config_index,
                        asc.config_mode,
                        &mut asc.sbr_config_changed,
                        1,
                    ) != 0
                    {
                        return TRANSPORTDEC_PARSE_ERROR;
                    }
                }
                // usc->element[elemIdx].m_stereoConfigIndex =
                fdk_read_bits(h_bs, 2);
                if e.m_stereo_config_index > 0 && cb.has_ssc() {
                    let mut samples_per_frame = asc.m_samples_per_frame as i32;
                    let ratio = asc.m_sc.m_usac_config.m_sbr_ratio_index;

                    if ratio == 1 {
                        samples_per_frame <<= 2;
                    }
                    if ratio == 2 {
                        samples_per_frame = (samples_per_frame * 8) / 3;
                    }
                    if ratio == 3 {
                        samples_per_frame <<= 1;
                    }

                    error_status = cb.ssc(
                        h_bs,
                        AOT_DRM_USAC, // syntax differs from MPEG Mps212Config()
                        asc.m_extension_sampling_frequency as i32,
                        samples_per_frame,
                        1, // only downmix channels (residual channels are not counted)
                        e.m_stereo_config_index as i32,
                        asc.m_sc.m_usac_config.m_core_sbr_frame_length_index as i32,
                        0, // don't know the length
                        asc.config_mode,
                        &mut asc.sac_config_changed,
                    );
                    // (cbSsc == NULL: ErrorStatus = TRANSPORTDEC_UNSUPPORTED_FORMAT is commented
                    // out in C.)
                }
            }
        }
        _ => return TRANSPORTDEC_PARSE_ERROR,
    }

    error_status
}

/// `Drm_xHEAACStaticConfig(asc, bs, audioMode, cb)`: `cb == None` signals the config check only
/// mode.
pub fn drm_xhe_aac_static_config(
    asc: &mut CSAudioSpecificConfig,
    bs: &mut FdkBitStream,
    audio_mode: i32,
    cb: Option<&mut dyn TpDecCallbacks>,
) -> TransportDecError {
    let core_sbr_frame_length_index_drm = fdk_read_bits(bs, 2) as i32;
    if usac_config_set_core_sbr_frame_length_index(asc, core_sbr_frame_length_index_drm + 1)
        != TRANSPORTDEC_OK
    {
        return TRANSPORTDEC_PARSE_ERROR;
    }

    asc.m_channel_configuration = if audio_mode != 0 { 2 } else { 1 };

    if drm_xhe_aac_decoder_config(asc, bs, audio_mode, cb) != TRANSPORTDEC_OK {
        return TRANSPORTDEC_PARSE_ERROR;
    }

    TRANSPORTDEC_OK
}

/// `mapSr2MPEGIdx[]`: mapping of the DRM audio sampling rate field to the MPEG
/// usacSamplingFrequencyIndex.
pub static MAP_SR2_MPEG_IDX: [u8; 8] = [
    0x1b, //  9.6 kHz
    0x09, // 12.0 kHz
    0x08, // 16.0 kHz
    0x17, // 19.2 kHz
    0x06, // 24.0 kHz
    0x05, // 32.0 kHz
    0x12, // 38.4 kHz
    0x03, // 48.0 kHz
];

/// `DrmRawSdcAudioConfig_Parse(self, bs, cb, configMode, configChanged)`: parses a DRM SDC audio
/// config (audio information data entity, type 9). `cb == None` signals the config check only
/// mode.
pub fn drm_raw_sdc_audio_config_parse(
    self_: &mut CSAudioSpecificConfig,
    bs: &mut FdkBitStream,
    cb: Option<&mut dyn TpDecCallbacks>,
    config_mode: u8,
    config_changed: u8,
) -> TransportDecError {
    let mut error_status = TRANSPORTDEC_OK;

    audio_specific_config_init(self_);

    if (fdk_get_valid_bits(bs) as i32) < 16 {
        return TRANSPORTDEC_PARSE_ERROR;
    }

    // DRM - Audio information data entity - type 9
    // - Short Id            2 bits (not part of the config buffer)
    // - Stream Id           2 bits (not part of the config buffer)
    // - audio coding        2 bits
    // - SBR flag            1 bit
    // - audio mode          2 bits
    // - audio sampling rate 3 bits
    // - text flag           1 bit
    // - enhancement flag    1 bit
    // - coder field         5 bits
    // - rfa                 1 bit

    self_.config_mode = config_mode;
    self_.aac_config_changed = config_changed;
    self_.sbr_config_changed = config_changed;
    self_.sac_config_changed = config_changed;

    // Read the SDC field
    let audio_coding = fdk_read_bits(bs, 2) as i32;
    let mut sbr_flag = fdk_read_bits(bs, 1) as i32;
    let audio_mode = fdk_read_bits(bs, 2) as i32;
    let c_sampling_freq = fdk_read_bits(bs, 3) as i32; // audio sampling rate

    fdk_read_bits(bs, 2); // Text and enhancement flag
    let coder_field = fdk_read_bits(bs, 5) as i32;
    fdk_read_bits(bs, 1); // rfa

    // Evaluate configuration and fill the ASC
    let sf_idx: i32;
    if audio_coding == 3 {
        sf_idx = MAP_SR2_MPEG_IDX[c_sampling_freq as usize] as i32;
        sbr_flag = 0; // rfa
    } else {
        sf_idx = match c_sampling_freq {
            0 => 11, //  8 kHz
            1 => 9,  // 12 kHz
            2 => 8,  // 16 kHz
            3 => 6,  // 24 kHz
            5 => 3,  // 48 kHz
            // 4, 6, 7: reserved
            _ => return TRANSPORTDEC_PARSE_ERROR,
        };
    }

    self_.m_sampling_frequency_index = sf_idx as u8;
    self_.m_sampling_frequency = sampling_rate_table(sf_idx as usize);

    if sbr_flag != 0 {
        let mut tmp: i32 = -1;
        self_.m_sbr_present_flag = 1;
        self_.m_extension_audio_object_type = AOT_SBR;
        self_.m_extension_sampling_frequency = self_.m_sampling_frequency << 1;
        for (i, &sr) in SAMPLING_RATE_TABLE.iter().enumerate() {
            if sr == self_.m_extension_sampling_frequency {
                tmp = i as i32;
                break;
            }
        }
        self_.m_extension_sampling_frequency_index = tmp as u8;
    }

    match audio_coding {
        0 => {
            // AAC
            if (coder_field >> 2) != 0 && audio_mode != 1 {
                self_.m_aot = AOT_DRM_SURROUND; // Set pseudo AOT for Drm Surround
            } else {
                self_.m_aot = AOT_DRM_AAC; // Set pseudo AOT for Drm AAC
            }
            match audio_mode {
                1 | 0 => {
                    if audio_mode == 1 {
                        // parametric stereo
                        self_.m_ps_present_flag = 1;
                    }
                    // mono
                    self_.m_channel_configuration = 1;
                }
                2 => {
                    // stereo
                    self_.m_channel_configuration = 2;
                }
                _ => return TRANSPORTDEC_PARSE_ERROR,
            }
            self_.m_vcb11_flag = 1;
            self_.m_hcr_flag = 1;
            self_.m_samples_per_frame = 960;
            self_.m_ep_config = 1;
        }
        1 => {
            // CELP
            self_.m_aot = AOT_ER_CELP;
            self_.m_channel_configuration = 1;
        }
        2 => {
            // HVXC
            self_.m_aot = AOT_ER_HVXC;
            self_.m_channel_configuration = 1;
        }
        3 => {
            // xHE-AAC: payload is MPEG conform -> no pseudo DRM AOT needed
            self_.m_aot = AOT_USAC;
            match audio_mode {
                0 | 2 => {
                    // mono, stereo: codec specific config 8n bits
                    error_status = drm_xhe_aac_static_config(self_, bs, audio_mode, cb);
                }
                _ => return TRANSPORTDEC_PARSE_ERROR,
            }
        }
        _ => {
            error_status = TRANSPORTDEC_PARSE_ERROR;
            self_.m_aot = AOT_NONE;
        }
    }

    if self_.m_ps_present_flag != 0 && self_.m_sbr_present_flag == 0 {
        return TRANSPORTDEC_PARSE_ERROR;
    }

    error_status
}
