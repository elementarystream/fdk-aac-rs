// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libMpegTPEnc/src/tpenc_asc.h, libMpegTPEnc/src/tpenc_asc.cpp.

//! Port of `tpenc_asc.h`: writing the AudioSpecificConfig (GA, ELD and USAC specific
//! configurations, explicit SBR/PS signaling) and the program config element.
//!
//! `transportEnc_GetChannelMode()` and `transportEnc_writeASC()` are declared in `tpenc_lib.h`
//! but defined here; [`crate::tpenc_lib`] re-exports them.
//!
//! The code is identical in the `aarch64` and `x86` reference builds (the preprocessed
//! translation units differ only in `FDK_ASSERT`, and the x86 one computes the unused
//! `bottomBackEnd` of an assertion).

use fdk_aac_rs_fdk::audio::*;
use fdk_aac_rs_fdk::bitbuffer::BitMem;
use fdk_aac_rs_fdk::bitstream::{
    fdk_byte_align, fdk_get_valid_bits, fdk_init_bit_stream, fdk_read_bit, fdk_sync_cache,
    fdk_write_bits, FdkBitStream, BS_READER,
};
use fdk_aac_rs_fdk::crc::{
    fdk_crc_end_reg, fdk_crc_get_crc, fdk_crc_init, fdk_crc_start_reg, FdkCrcInfo,
};
use fdk_aac_rs_fdk::fdk_assert;
use fdk_aac_rs_tpdec::tp_data::{
    get_sampling_rate_index, ASCEXT_PS, ASCEXT_SBR, ASCEXT_UNKOWN, ELDEXT_DOWNSCALEINFO,
    ELDEXT_LDSAC, ELDEXT_TERM,
};

use crate::tpenc_lib::{TpEncCallbacks, TRANSPORTENC_OK, TRANSPORTENC_UNSUPPORTED_FORMAT};

/// `PCE_HEIGHT_EXT_SYNC`
pub const PCE_HEIGHT_EXT_SYNC: u32 = 0xAC;
/// `HEIGHT_NORMAL`
pub const HEIGHT_NORMAL: u32 = 0;
/// `HEIGHT_TOP`
pub const HEIGHT_TOP: u32 = 1;
/// `HEIGHT_BOTTOM`
pub const HEIGHT_BOTTOM: u32 = 2;
/// `MAX_FRONT_ELEMENTS`
pub const MAX_FRONT_ELEMENTS: usize = 8;
/// `MAX_SIDE_ELEMENTS`
pub const MAX_SIDE_ELEMENTS: usize = 3;
/// `MAX_BACK_ELEMENTS`
pub const MAX_BACK_ELEMENTS: usize = 4;

// ---------------------------------------------------------------------------------------------
// PCE configuration table
// ---------------------------------------------------------------------------------------------

/// `PCE_HEIGHT_NUM`: number of front, side and back channel elements in the top `[0]` and
/// bottom `[1]` plane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PceHeightNum {
    pub num_front_height_channel_elements: [u8; 2],
    pub num_side_height_channel_elements: [u8; 2],
    pub num_back_height_channel_elements: [u8; 2],
}

/// `PCE_CONFIGURATION`: a PCE described by the placed channel elements and the element type
/// sequence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PceConfiguration {
    pub num_front_channel_elements: u8,
    pub num_side_channel_elements: u8,
    pub num_back_channel_elements: u8,
    pub num_lfe_channel_elements: u8,
    /// `pEl_type`: the sequence of the elements of the channel mode (MPEG order).
    pub p_el_type: &'static [Mp4ElementId],
    /// `pHeight_num` (`NULL`: no height information).
    pub p_height_num: Option<&'static PceHeightNum>,
}

/// `CHANNEL_CONFIGURATION`: maps a channel mode to its PCE configuration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChannelConfiguration {
    pub channel_mode: ChannelMode,
    pub pce_configuration: PceConfiguration,
}

// The element type sequences and the height table, dumped with scripts/dump_table.py (the command
// is given at each table).
/// `elType_1`
///
/// `scripts/dump_table.py libMpegTPEnc/src/tpenc_asc.cpp elType_1 INT --name ELTYPE_1`
pub static ELTYPE_1: [i32; 1] = [0];
/// `elType_2`
///
/// `scripts/dump_table.py libMpegTPEnc/src/tpenc_asc.cpp elType_2 INT --name ELTYPE_2`
pub static ELTYPE_2: [i32; 1] = [1];
/// `elType_1_2`
///
/// `scripts/dump_table.py libMpegTPEnc/src/tpenc_asc.cpp elType_1_2 INT --name ELTYPE_1_2`
pub static ELTYPE_1_2: [i32; 2] = [0, 1];
/// `elType_1_2_1`
///
/// `scripts/dump_table.py libMpegTPEnc/src/tpenc_asc.cpp elType_1_2_1 INT --name ELTYPE_1_2_1`
pub static ELTYPE_1_2_1: [i32; 3] = [0, 1, 0];
/// `elType_1_2_2`
///
/// `scripts/dump_table.py libMpegTPEnc/src/tpenc_asc.cpp elType_1_2_2 INT --name ELTYPE_1_2_2`
pub static ELTYPE_1_2_2: [i32; 3] = [0, 1, 1];
/// `elType_1_2_2_1`
///
/// `scripts/dump_table.py libMpegTPEnc/src/tpenc_asc.cpp elType_1_2_2_1 INT --name ELTYPE_1_2_2_1`
pub static ELTYPE_1_2_2_1: [i32; 4] = [0, 1, 1, 3];
/// `elType_1_2_2_2_1`
///
/// `scripts/dump_table.py libMpegTPEnc/src/tpenc_asc.cpp elType_1_2_2_2_1 INT --name ELTYPE_1_2_2_2_1`
pub static ELTYPE_1_2_2_2_1: [i32; 5] = [0, 1, 1, 1, 3];
/// `elType_6_1`
///
/// `scripts/dump_table.py libMpegTPEnc/src/tpenc_asc.cpp elType_6_1 INT --name ELTYPE_6_1`
pub static ELTYPE_6_1: [i32; 5] = [0, 1, 1, 0, 3];
/// `elType_7_1_back`
///
/// `scripts/dump_table.py libMpegTPEnc/src/tpenc_asc.cpp elType_7_1_back INT --name ELTYPE_7_1_BACK`
pub static ELTYPE_7_1_BACK: [i32; 5] = [0, 1, 1, 1, 3];
/// `elType_7_1_top_front`
///
/// `scripts/dump_table.py libMpegTPEnc/src/tpenc_asc.cpp elType_7_1_top_front INT --name ELTYPE_7_1_TOP_FRONT`
pub static ELTYPE_7_1_TOP_FRONT: [i32; 5] = [0, 1, 1, 3, 1];
/// `elType_7_1_rear_surround`
///
/// `scripts/dump_table.py libMpegTPEnc/src/tpenc_asc.cpp elType_7_1_rear_surround INT --name ELTYPE_7_1_REAR_SURROUND`
pub static ELTYPE_7_1_REAR_SURROUND: [i32; 5] = [0, 1, 1, 1, 3];
/// `elType_7_1_front_center`
///
/// `scripts/dump_table.py libMpegTPEnc/src/tpenc_asc.cpp elType_7_1_front_center INT --name ELTYPE_7_1_FRONT_CENTER`
pub static ELTYPE_7_1_FRONT_CENTER: [i32; 5] = [0, 1, 1, 1, 3];
/// `heightNum_7_1_top_front` as its six bytes.
///
/// `scripts/dump_table.py libMpegTPEnc/src/tpenc_asc.cpp heightNum_7_1_top_front UCHAR --name HEIGHT_NUM_7_1_TOP_FRONT`
pub static HEIGHT_NUM_7_1_TOP_FRONT: [u8; 6] = [1, 0, 0, 0, 0, 0];

static HEIGHT_NUM_7_1_TOP_FRONT_S: PceHeightNum = PceHeightNum {
    num_front_height_channel_elements: [HEIGHT_NUM_7_1_TOP_FRONT[0], HEIGHT_NUM_7_1_TOP_FRONT[1]],
    num_side_height_channel_elements: [HEIGHT_NUM_7_1_TOP_FRONT[2], HEIGHT_NUM_7_1_TOP_FRONT[3]],
    num_back_height_channel_elements: [HEIGHT_NUM_7_1_TOP_FRONT[4], HEIGHT_NUM_7_1_TOP_FRONT[5]],
};

/// The element type sequences in the order of `PCE_CONFIG_TAB_FLAT` column 5 (the order of the
/// array `t` of the prelude of its dump; not a table of the C++ code).
pub static ELTYPES: [&[i32]; 12] = [
    &ELTYPE_1,
    &ELTYPE_2,
    &ELTYPE_1_2,
    &ELTYPE_1_2_1,
    &ELTYPE_1_2_2,
    &ELTYPE_1_2_2_1,
    &ELTYPE_1_2_2_2_1,
    &ELTYPE_6_1,
    &ELTYPE_7_1_BACK,
    &ELTYPE_7_1_TOP_FRONT,
    &ELTYPE_7_1_REAR_SURROUND,
    &ELTYPE_7_1_FRONT_CENTER,
];

/// `pceConfigTab[]` flattened, 7 values per entry: channel mode, the numbers of front, side,
/// back and LFE elements, the index of `pEl_type` in [`ELTYPES`] and `pHeight_num` (0: `NULL`,
/// 1: `&heightNum_7_1_top_front`). Dumped with (a constructor in the prelude flattens the table
/// and maps the pointers to indices)
///
/// `scripts/dump_table.py libMpegTPEnc/src/tpenc_asc.cpp pce_flat INT --per-line 7 --name PCE_CONFIG_TAB_FLAT --prelude 'static INT pce_flat[sizeof(pceConfigTab) / sizeof(pceConfigTab[0]) * 7]; static int pce_init = [] { const MP4_ELEMENT_ID *t[12] = {elType_1, elType_2, elType_1_2, elType_1_2_1, elType_1_2_2, elType_1_2_2_1, elType_1_2_2_2_1, elType_6_1, elType_7_1_back, elType_7_1_top_front, elType_7_1_rear_surround, elType_7_1_front_center}; for (int i = 0; i < (int)(sizeof(pceConfigTab) / sizeof(pceConfigTab[0])); i++) { const PCE_CONFIGURATION *c = &pceConfigTab[i].pce_configuration; int k = -1; for (int j = 0; j < 12; j++) if (c->pEl_type == t[j]) k = j; pce_flat[7 * i] = pceConfigTab[i].channel_mode; pce_flat[7 * i + 1] = c->num_front_channel_elements; pce_flat[7 * i + 2] = c->num_side_channel_elements; pce_flat[7 * i + 3] = c->num_back_channel_elements; pce_flat[7 * i + 4] = c->num_lfe_channel_elements; pce_flat[7 * i + 5] = k; pce_flat[7 * i + 6] = c->pHeight_num == NULL ? 0 : (c->pHeight_num == &heightNum_7_1_top_front ? 1 : -1); } return 0; }();'`
pub static PCE_CONFIG_TAB_FLAT: [i32; 84] = [
    1, 1, 0, 0, 0, 0, 0, //
    2, 1, 0, 0, 0, 1, 0, //
    3, 2, 0, 0, 0, 2, 0, //
    4, 2, 0, 1, 0, 3, 0, //
    5, 2, 0, 1, 0, 4, 0, //
    6, 2, 0, 1, 1, 5, 0, //
    7, 3, 0, 1, 1, 6, 0, //
    11, 2, 0, 2, 1, 7, 0, //
    12, 2, 0, 2, 1, 8, 0, //
    14, 3, 0, 1, 1, 9, 1, //
    33, 2, 0, 2, 1, 10, 0, //
    34, 3, 0, 1, 1, 11, 0, //
];

const fn pce_entry(i: usize) -> ChannelConfiguration {
    let v = &PCE_CONFIG_TAB_FLAT;
    ChannelConfiguration {
        channel_mode: v[i * 7],
        pce_configuration: PceConfiguration {
            num_front_channel_elements: v[i * 7 + 1] as u8,
            num_side_channel_elements: v[i * 7 + 2] as u8,
            num_back_channel_elements: v[i * 7 + 3] as u8,
            num_lfe_channel_elements: v[i * 7 + 4] as u8,
            p_el_type: ELTYPES[v[i * 7 + 5] as usize],
            p_height_num: if v[i * 7 + 6] == 1 { Some(&HEIGHT_NUM_7_1_TOP_FRONT_S) } else { None },
        },
    }
}

/// `pceConfigTab[]`: all supported channel modes and their PCE configuration.
pub static PCE_CONFIG_TAB: [ChannelConfiguration; 12] = [
    pce_entry(0),
    pce_entry(1),
    pce_entry(2),
    pce_entry(3),
    pce_entry(4),
    pce_entry(5),
    pce_entry(6),
    pce_entry(7),
    pce_entry(8),
    pce_entry(9),
    pce_entry(10),
    pce_entry(11),
];

/// `getPceEntry(channel_mode)`: the PCE configuration of a channel mode, `None` (`NULL`) if the
/// mode has none.
pub fn get_pce_entry(channel_mode: ChannelMode) -> Option<&'static PceConfiguration> {
    PCE_CONFIG_TAB.iter().find(|e| e.channel_mode == channel_mode).map(|e| &e.pce_configuration)
}

/// `getChannelConfig(channel_mode, channel_config_zero)`: the channel configuration of a channel
/// mode (0 if none, or if `channel_config_zero` is set).
pub fn get_channel_config(channel_mode: ChannelMode, channel_config_zero: u8) -> i32 {
    if channel_config_zero != 0 {
        0
    } else {
        match channel_mode {
            MODE_1 => 1,
            MODE_2 => 2,
            MODE_1_2 => 3,
            MODE_1_2_1 => 4,
            MODE_1_2_2 => 5,
            MODE_1_2_2_1 => 6,
            MODE_1_2_2_2_1 => 7,
            MODE_6_1 => 11,
            MODE_7_1_BACK => 12,
            MODE_7_1_TOP_FRONT => 14,
            _ => 0,
        }
    }
}

/// `transportEnc_GetChannelMode(noChannels)`: a reasonable channel mode for a channel count.
pub fn transport_enc_get_channel_mode(no_channels: i32) -> ChannelMode {
    if no_channels <= 8 && no_channels > 0 {
        // see : iso/mpeg4 v1 audio subpart1
        if no_channels == 8 {
            7
        } else {
            no_channels
        }
    } else {
        MODE_UNKNOWN
    }
}

/// Element depth of `transportEnc_writePCE()`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ElementDepth {
    Front,
    Side,
    Back,
}

/// `transportEnc_writePCE(hBs, channelMode, sampleRate, instanceTagPCE, profile,
/// matrixMixdownA, pseudoSurroundEnable, alignAnchor)`: writes a program config element.
/// Returns 0 on success, -1 for a sampling rate without index or a channel mode without PCE
/// configuration (nothing is written then).
#[allow(clippy::too_many_arguments)]
pub fn transport_enc_write_pce(
    h_bs: &mut FdkBitStream,
    channel_mode: ChannelMode,
    sample_rate: i32,
    instance_tag_pce: i32,
    profile: i32,
    matrix_mixdown_a: i32,
    pseudo_surround_enable: i32,
    align_anchor: u32,
) -> i32 {
    let (mut cpe_cnt, mut sce_cnt, mut lfe_cnt): (u8, u8, u8) = (0, 0, 0);
    let (mut frnt_cnt, mut sd_cnt, mut bck_cnt): (u8, u8, u8) = (0, 0, 0);
    let (normal_front_end, normal_side_end, normal_back_end): (u8, u8, u8);
    let (mut top_front_end, mut top_side_end, mut top_back_end): (u8, u8, u8) = (0, 0, 0);
    let (mut bottom_front_end, mut bottom_side_end): (u8, u8) = (0, 0);

    let sample_rate_index = get_sampling_rate_index(sample_rate as u32, 4);
    if sample_rate_index == 15 {
        return -1;
    }

    let Some(config) = get_pce_entry(channel_mode) else {
        return -1;
    };

    fdk_assert!(config.num_front_channel_elements as usize <= MAX_FRONT_ELEMENTS);
    fdk_assert!(config.num_side_channel_elements as usize <= MAX_SIDE_ELEMENTS);
    fdk_assert!(config.num_back_channel_elements as usize <= MAX_BACK_ELEMENTS);

    let mut front_is_cpe = [0u8; MAX_FRONT_ELEMENTS];
    let mut front_tag = [0u8; MAX_FRONT_ELEMENTS];
    let mut side_is_cpe = [0u8; MAX_SIDE_ELEMENTS];
    let mut side_tag = [0u8; MAX_SIDE_ELEMENTS];
    let mut back_is_cpe = [0u8; MAX_BACK_ELEMENTS];
    let mut back_tag = [0u8; MAX_BACK_ELEMENTS];

    // Write general information

    fdk_write_bits(h_bs, instance_tag_pce as u32, 4); // Element instance tag
    fdk_write_bits(h_bs, profile as u32, 2); // Object type
    fdk_write_bits(h_bs, sample_rate_index as u32, 4); // Sample rate index

    fdk_write_bits(h_bs, config.num_front_channel_elements as u32, 4); // Front channel Elements
    fdk_write_bits(h_bs, config.num_side_channel_elements as u32, 4); // No Side Channel Elements
    fdk_write_bits(h_bs, config.num_back_channel_elements as u32, 4); // No Back channel Elements
    fdk_write_bits(h_bs, config.num_lfe_channel_elements as u32, 2); // No Lfe channel elements

    fdk_write_bits(h_bs, 0, 3); // No assoc data elements
    fdk_write_bits(h_bs, 0, 4); // No valid cc elements
    fdk_write_bits(h_bs, 0, 1); // Mono mixdown present
    fdk_write_bits(h_bs, 0, 1); // Stereo mixdown present

    if matrix_mixdown_a != 0 && (channel_mode == MODE_1_2_2 || channel_mode == MODE_1_2_2_1) {
        fdk_write_bits(h_bs, 1, 1); // Matrix mixdown present
        fdk_write_bits(h_bs, (matrix_mixdown_a.wrapping_sub(1) & 0x3) as u32, 2); // matrix_mixdown_idx
        fdk_write_bits(h_bs, (pseudo_surround_enable != 0) as u32, 1); // pseudo_surround_enable
    } else {
        fdk_write_bits(h_bs, 0, 1); // Matrix mixdown not present
    }

    if let Some(h) = config.p_height_num {
        // we have up to three different height levels, and in each height level we may have
        // front, side and back channels. We need to know where each section ends to correctly
        // count the tags (UCHAR arithmetic as in C)
        normal_front_end = config
            .num_front_channel_elements
            .wrapping_sub(h.num_front_height_channel_elements[0])
            .wrapping_sub(h.num_front_height_channel_elements[1]);
        normal_side_end = normal_front_end
            .wrapping_add(config.num_side_channel_elements)
            .wrapping_sub(h.num_side_height_channel_elements[0])
            .wrapping_sub(h.num_side_height_channel_elements[1]);
        normal_back_end = normal_side_end
            .wrapping_add(config.num_back_channel_elements)
            .wrapping_sub(h.num_back_height_channel_elements[0])
            .wrapping_sub(h.num_back_height_channel_elements[1]);

        // only normal height LFEs assumed
        top_front_end = normal_back_end
            .wrapping_add(config.num_lfe_channel_elements)
            .wrapping_add(h.num_front_height_channel_elements[0]);
        top_side_end = top_front_end.wrapping_add(h.num_side_height_channel_elements[0]);
        top_back_end = top_side_end.wrapping_add(h.num_back_height_channel_elements[0]);

        bottom_front_end = top_back_end.wrapping_add(h.num_front_height_channel_elements[1]);
        bottom_side_end = bottom_front_end.wrapping_add(h.num_side_height_channel_elements[1]);
    } else {
        // we have only one height level, so we don't care about top or bottom
        normal_front_end = config.num_front_channel_elements;
        normal_side_end = normal_front_end.wrapping_add(config.num_side_channel_elements);
        normal_back_end = normal_side_end.wrapping_add(config.num_back_channel_elements);
    }

    // assign cpe and tag information to either front, side or back channels

    let n_el = config.num_front_channel_elements as i32
        + config.num_side_channel_elements as i32
        + config.num_back_channel_elements as i32
        + config.num_lfe_channel_elements as i32;
    let mut p_el_list = config.p_el_type.iter();
    for i in 0..n_el {
        // The table is consistent: the sequence has one entry per element.
        let el = p_el_list.next().copied().unwrap_or(ID_NONE);
        if el == ID_LFE {
            continue;
        }
        let is_cpe: u8 = (el == ID_CPE) as u8;
        let tag = if is_cpe != 0 {
            let t = cpe_cnt;
            cpe_cnt = cpe_cnt.wrapping_add(1);
            t
        } else {
            let t = sce_cnt;
            sce_cnt = sce_cnt.wrapping_add(1);
            t
        };

        let el_depth = if i < normal_front_end as i32 {
            ElementDepth::Front
        } else if i < normal_side_end as i32 {
            ElementDepth::Side
        } else if i < normal_back_end as i32 {
            ElementDepth::Back
        } else if i < top_front_end as i32 {
            ElementDepth::Front
        } else if i < top_side_end as i32 {
            ElementDepth::Side
        } else if i < top_back_end as i32 {
            ElementDepth::Back
        } else if i < bottom_front_end as i32 {
            ElementDepth::Front
        } else if i < bottom_side_end as i32 {
            ElementDepth::Side
        } else {
            // FDK_ASSERT(i < bottomBackEnd): won't fail if implementation of pce configuration
            // table is correct
            ElementDepth::Back
        };

        // The counters stay below the array sizes for the entries of the table.
        match el_depth {
            ElementDepth::Front => {
                fdk_assert!(frnt_cnt < config.num_front_channel_elements);
                if let Some(v) = front_is_cpe.get_mut(frnt_cnt as usize) {
                    *v = is_cpe;
                }
                if let Some(v) = front_tag.get_mut(frnt_cnt as usize) {
                    *v = tag;
                }
                frnt_cnt = frnt_cnt.wrapping_add(1);
            }
            ElementDepth::Side => {
                fdk_assert!(sd_cnt < config.num_side_channel_elements);
                if let Some(v) = side_is_cpe.get_mut(sd_cnt as usize) {
                    *v = is_cpe;
                }
                if let Some(v) = side_tag.get_mut(sd_cnt as usize) {
                    *v = tag;
                }
                sd_cnt = sd_cnt.wrapping_add(1);
            }
            ElementDepth::Back => {
                fdk_assert!(bck_cnt < config.num_back_channel_elements);
                if let Some(v) = back_is_cpe.get_mut(bck_cnt as usize) {
                    *v = is_cpe;
                }
                if let Some(v) = back_tag.get_mut(bck_cnt as usize) {
                    *v = tag;
                }
                bck_cnt = bck_cnt.wrapping_add(1);
            }
        }
    }

    // Write front channel isCpe and tags
    for i in 0..config.num_front_channel_elements as usize {
        fdk_write_bits(h_bs, front_is_cpe[i] as u32, 1);
        fdk_write_bits(h_bs, front_tag[i] as u32, 4);
    }
    // Write side channel isCpe and tags
    for i in 0..config.num_side_channel_elements as usize {
        fdk_write_bits(h_bs, side_is_cpe[i] as u32, 1);
        fdk_write_bits(h_bs, side_tag[i] as u32, 4);
    }
    // Write back channel isCpe and tags
    for i in 0..config.num_back_channel_elements as usize {
        fdk_write_bits(h_bs, back_is_cpe[i] as u32, 1);
        fdk_write_bits(h_bs, back_tag[i] as u32, 4);
    }
    // Write LFE information
    for _ in 0..config.num_lfe_channel_elements {
        fdk_write_bits(h_bs, lfe_cnt as u32, 4); // LFE channel Instance Tag.
        lfe_cnt = lfe_cnt.wrapping_add(1);
    }

    // - num_valid_cc_elements always 0.
    // - num_assoc_data_elements always 0.

    // Byte alignment: relative to alignAnchor
    //   ADTS: align with respect to the first bit of the raw_data_block()
    //   ADIF: align with respect to the first bit of the header
    //   LATM: align with respect to the first bit of the ASC
    fdk_byte_align(h_bs, align_anchor); // Alignment

    // Write comment information

    if let Some(h) = config.p_height_num {
        // embed height information in comment field

        let comment_bytes: i32 = 1 // PCE_HEIGHT_EXT_SYNC
            + ((((config.num_front_channel_elements as i32
                + config.num_side_channel_elements as i32
                + config.num_back_channel_elements as i32)
                << 1)
                + 7)
                >> 3) // 2 bit height info per element, round up to full bytes
            + 1; // CRC

        fdk_write_bits(h_bs, comment_bytes as u32, 8); // comment size.

        let mut crc_info = FdkCrcInfo::default(); // CRC state info

        fdk_crc_init(&mut crc_info, 0x07, 0xFF, 8);
        let crc_reg = fdk_crc_start_reg(&mut crc_info, h_bs, 0);

        fdk_write_bits(h_bs, PCE_HEIGHT_EXT_SYNC, 8); // indicate height extension

        // front channel height information
        let n = config.num_front_channel_elements as i32
            - h.num_front_height_channel_elements[0] as i32
            - h.num_front_height_channel_elements[1] as i32;
        for _ in 0..n {
            fdk_write_bits(h_bs, HEIGHT_NORMAL, 2);
        }
        for _ in 0..h.num_front_height_channel_elements[0] {
            fdk_write_bits(h_bs, HEIGHT_TOP, 2);
        }
        for _ in 0..h.num_front_height_channel_elements[1] {
            fdk_write_bits(h_bs, HEIGHT_BOTTOM, 2);
        }

        // side channel height information
        let n = config.num_side_channel_elements as i32
            - h.num_side_height_channel_elements[0] as i32
            - h.num_side_height_channel_elements[1] as i32;
        for _ in 0..n {
            fdk_write_bits(h_bs, HEIGHT_NORMAL, 2);
        }
        for _ in 0..h.num_side_height_channel_elements[0] {
            fdk_write_bits(h_bs, HEIGHT_TOP, 2);
        }
        for _ in 0..h.num_side_height_channel_elements[1] {
            fdk_write_bits(h_bs, HEIGHT_BOTTOM, 2);
        }

        // back channel height information
        let n = config.num_back_channel_elements as i32
            - h.num_back_height_channel_elements[0] as i32
            - h.num_back_height_channel_elements[1] as i32;
        for _ in 0..n {
            fdk_write_bits(h_bs, HEIGHT_NORMAL, 2);
        }
        for _ in 0..h.num_back_height_channel_elements[0] {
            fdk_write_bits(h_bs, HEIGHT_TOP, 2);
        }
        for _ in 0..h.num_back_height_channel_elements[1] {
            fdk_write_bits(h_bs, HEIGHT_BOTTOM, 2);
        }

        fdk_byte_align(h_bs, align_anchor); // Alignment

        fdk_crc_end_reg(&mut crc_info, h_bs, crc_reg);
        fdk_write_bits(h_bs, fdk_crc_get_crc(&crc_info) as u32, 8);
    } else {
        fdk_write_bits(h_bs, 0, 8); // Do no write any comment or height information.
    }

    0
}

/// `transportEnc_GetPCEBits(channelMode, matrixMixdownA, bits)`: `bits` plus the bit count of a
/// program config element starting at bit offset `bits`; -1 for a channel mode without PCE
/// configuration.
pub fn transport_enc_get_pce_bits(
    channel_mode: ChannelMode,
    matrix_mixdown_a: i32,
    mut bits: i32,
) -> i32 {
    let Some(config) = get_pce_entry(channel_mode) else {
        return -1; // unsupported channelmapping
    };

    // The callers pass small offsets; wrapping as the reference build.
    bits = bits.wrapping_add(4 + 2 + 4); // Element instance tag + Object type + Sample rate index
    bits = bits.wrapping_add(4 + 4 + 4 + 2); // No (front + side + back + lfe channel) elements
    bits = bits.wrapping_add(3 + 4); // No (assoc data + valid cc) elements
    bits = bits.wrapping_add(1 + 1 + 1); // Mono + Stereo + Matrix mixdown present

    if matrix_mixdown_a != 0 && (channel_mode == MODE_1_2_2 || channel_mode == MODE_1_2_2_1) {
        bits = bits.wrapping_add(3); // matrix_mixdown_idx + pseudo_surround_enable
    }

    bits = bits.wrapping_add((1 + 4) * config.num_front_channel_elements as i32);
    bits = bits.wrapping_add((1 + 4) * config.num_side_channel_elements as i32);
    bits = bits.wrapping_add((1 + 4) * config.num_back_channel_elements as i32);
    bits = bits.wrapping_add(4 * config.num_lfe_channel_elements as i32);

    // - num_valid_cc_elements always 0.
    // - num_assoc_data_elements always 0.

    if (bits % 8) != 0 {
        bits = bits.wrapping_add(8 - (bits % 8)); // Alignment
    }

    bits = bits.wrapping_add(8); // Comment field  bytes

    if config.p_height_num.is_some() {
        // Comment field (height extension)

        bits = bits.wrapping_add(
            8 // PCE_HEIGHT_EXT_SYNC
            + ((config.num_front_channel_elements as i32
                + config.num_side_channel_elements as i32
                + config.num_back_channel_elements as i32)
                << 1) // 2 bit height info per element
            + 8, // CRC
        );

        if (bits % 8) != 0 {
            bits = bits.wrapping_add(8 - (bits % 8)); // Alignment
        }
    }

    bits
}

/// `writeAot(hBitstreamBuffer, aot)`
pub fn write_aot(h_bitstream_buffer: &mut FdkBitStream, aot: AudioObjectType) {
    let tmp = aot;

    if tmp > 31 {
        fdk_write_bits(h_bitstream_buffer, AOT_ESCAPE as u32, 5);
        fdk_write_bits(h_bitstream_buffer, tmp.wrapping_sub(32) as u32, 6); // AudioObjectType
    } else {
        fdk_write_bits(h_bitstream_buffer, tmp as u32, 5);
    }
}

/// `writeSampleRate(hBs, sampleRate, nBits)`
pub fn write_sample_rate(h_bs: &mut FdkBitStream, sample_rate: i32, n_bits: i32) {
    let sr_idx = get_sampling_rate_index(sample_rate as u32, n_bits as u32);

    fdk_write_bits(h_bs, sr_idx as u32, n_bits as u32);
    if sr_idx == (1i32.wrapping_shl(n_bits as u32)).wrapping_sub(1) {
        fdk_write_bits(h_bs, sample_rate as u32, 24);
    }
}

/// `transportEnc_writeGASpecificConfig(asc, config, extFlg, alignAnchor)`
pub fn transport_enc_write_ga_specific_config(
    asc: &mut FdkBitStream,
    config: &CoderConfig,
    ext_flg: i32,
    align_anchor: u32,
) -> i32 {
    let aot = config.aot;
    let samples_per_frame = config.samples_per_frame;

    // start of GASpecificConfig according to ISO/IEC 14496-3 Subpart 4, 4.4.1
    // frameLengthFlag: 1 for a 960/480 (I)MDCT, 0 for a 1024/512 (I)MDCT
    fdk_write_bits(asc, (samples_per_frame == 960 || samples_per_frame == 480) as u32, 1);
    // dependsOnCoreCoder: Sampling Rate Coder Specific, see in ISO/IEC 14496-3 Subpart 4, 4.4.1
    fdk_write_bits(asc, 0, 1);
    // Extension Flag: Shall be 1 for aot = 17,19,20,21,22,23
    fdk_write_bits(asc, ext_flg as u32, 1);

    // Write PCE if channel config is not 1-7
    if get_channel_config(config.channel_mode, config.channel_config_zero) == 0 {
        transport_enc_write_pce(
            asc,
            config.channel_mode,
            config.sampling_rate,
            0,
            1,
            config.matrix_mixdown_a as i32,
            ((config.flags & CC_PSEUDO_SURROUND) != 0) as i32,
            align_anchor,
        );
    }
    if aot == AOT_AAC_SCAL || aot == AOT_ER_AAC_SCAL {
        fdk_write_bits(asc, 0, 3); // layerNr
    }
    if ext_flg != 0 {
        if aot == AOT_ER_BSAC {
            fdk_write_bits(asc, config.bsac_num_of_sub_frame as u32, 5); // numOfSubFrame
            fdk_write_bits(asc, config.bsac_layer_length as u32, 11); // layer_length
        }
        if aot == AOT_ER_AAC_LC
            || aot == AOT_ER_AAC_LTP
            || aot == AOT_ER_AAC_SCAL
            || aot == AOT_ER_AAC_LD
        {
            // aacSectionDataResillienceFlag
            fdk_write_bits(asc, ((config.flags & CC_VCB11) != 0) as u32, 1);
            // aacScaleFactorDataResillienceFlag
            fdk_write_bits(asc, ((config.flags & CC_RVLC) != 0) as u32, 1);
            // aacSpectralDataResillienceFlag
            fdk_write_bits(asc, ((config.flags & CC_HCR) != 0) as u32, 1);
        }
        fdk_write_bits(asc, 0, 1); // extensionFlag3: reserved. Shall be '0'
    }
    0
}

/// `transportEnc_writeELDSpecificConfig(hBs, config, epConfig, cb)`
///
/// With `CC_SBR` and an SBR callback, the reference has no defined result for a channel mode
/// without a PCE configuration; the port then calls the callback for no element (PORTING.md,
/// "Deviations from the reference").
pub fn transport_enc_write_eld_specific_config(
    h_bs: &mut FdkBitStream,
    config: &CoderConfig,
    _ep_config: i32,
    cb: &mut dyn TpEncCallbacks,
) -> i32 {
    let frame_length_flag: u32 = match config.samples_per_frame {
        512 | 256 | 128 | 64 => 0,
        480 | 240 | 160 | 120 | 60 => 1,
        _ => 0,
    };

    fdk_write_bits(h_bs, frame_length_flag, 1);

    fdk_write_bits(h_bs, ((config.flags & CC_VCB11) != 0) as u32, 1);
    fdk_write_bits(h_bs, ((config.flags & CC_RVLC) != 0) as u32, 1);
    fdk_write_bits(h_bs, ((config.flags & CC_HCR) != 0) as u32, 1);

    fdk_write_bits(h_bs, ((config.flags & CC_SBR) != 0) as u32, 1); // SBR header flag
    if (config.flags & CC_SBR) != 0 {
        // Samplerate Flag
        fdk_write_bits(h_bs, (config.sampling_rate != config.ext_sampling_rate) as u32, 1);
        fdk_write_bits(h_bs, ((config.flags & CC_SBRCRC) != 0) as u32, 1); // SBR CRC flag

        if cb.has_sbr() {
            let mut sbr_element_index: i32 = 0;

            // (a channel mode without PCE configuration: deviation, see above)
            if let Some(p_pce) = get_pce_entry(config.channel_mode) {
                let n = p_pce.num_front_channel_elements as usize
                    + p_pce.num_side_channel_elements as usize
                    + p_pce.num_back_channel_elements as usize
                    + p_pce.num_lfe_channel_elements as usize;
                for e in 0..n {
                    let el = p_pce.p_el_type.get(e).copied().unwrap_or(ID_NONE);
                    if el == ID_SCE || el == ID_CPE {
                        cb.sbr(h_bs, 0, 0, 0, config.aot, el, sbr_element_index, 0, 0, 0, None, 1);
                        sbr_element_index += 1;
                    }
                }
            }
        }
    }

    if (config.flags & CC_SAC) != 0 && cb.has_ssc() {
        fdk_write_bits(h_bs, ELDEXT_LDSAC, 4);

        let eld_ext_len: i32 = cb
            .ssc(None, config.aot, config.ext_sampling_rate, 0, 0, 0, 0, 0, 0, None)
            .wrapping_add(7)
            >> 3;
        let mut cnt = eld_ext_len;

        if cnt < 0xF {
            fdk_write_bits(h_bs, cnt as u32, 4);
        } else {
            fdk_write_bits(h_bs, 0xF, 4);
            cnt -= 0xF;

            if cnt < 0xFF {
                fdk_write_bits(h_bs, cnt as u32, 8);
            } else {
                fdk_write_bits(h_bs, 0xFF, 8);
                cnt -= 0xFF;

                fdk_assert!(cnt <= 0xFFFF);
                fdk_write_bits(h_bs, cnt as u32, 16);
            }
        }

        cb.ssc(Some(h_bs), config.aot, config.ext_sampling_rate, 0, 0, 0, 0, 0, 0, None);
    }

    if config.downscale_sampling_rate != 0
        && config.downscale_sampling_rate != config.ext_sampling_rate
    {
        // downscale active

        // eldExtLenDsc: Number of bytes for the ELD downscale extension (srIdx needs 1 byte
        // + downscaleSamplingRate needs additional 3 bytes)
        let mut eld_ext_len_dsc: i32 = 1;
        let downscale_sampling_rate = config.downscale_sampling_rate;
        fdk_write_bits(h_bs, ELDEXT_DOWNSCALEINFO, 4); // ELDEXT_DOWNSCALEINFO

        if downscale_sampling_rate != 96000
            && downscale_sampling_rate != 88200
            && downscale_sampling_rate != 64000
            && downscale_sampling_rate != 48000
            && downscale_sampling_rate != 44100
            && downscale_sampling_rate != 32000
            && downscale_sampling_rate != 24000
            && downscale_sampling_rate != 22050
            && downscale_sampling_rate != 16000
            && downscale_sampling_rate != 12000
            && downscale_sampling_rate != 11025
            && downscale_sampling_rate != 8000
            && downscale_sampling_rate != 7350
        {
            // length extends to 4 if downscaleSamplingRate's value is not one of the listed
            // values
            eld_ext_len_dsc = 4;
        }

        fdk_write_bits(h_bs, eld_ext_len_dsc as u32, 4);
        write_sample_rate(h_bs, downscale_sampling_rate, 4);
        fdk_write_bits(h_bs, 0x0, 4); // fill_nibble
    }

    fdk_write_bits(h_bs, ELDEXT_TERM, 4); // ELDEXT_TERM

    0
}

/// `transportEnc_writeUsacSpecificConfig(hBs, extFlag, cc, cb)`: copies the raw USAC config
/// (`rawConfig`, `rawConfigBits` bits). Returns `TRANSPORTENC_UNSUPPORTED_FORMAT` if the length
/// is 0 or exceeds `rawConfig`.
///
/// C reads `rawConfig` through a bitstream of `BUFSIZE_DUMMY_VALUE` bytes; the bits written come
/// from `rawConfig` only. The port reads a zero-padded copy.
pub fn transport_enc_write_usac_specific_config(
    h_bs: &mut FdkBitStream,
    _ext_flag: i32,
    cc: &CoderConfig,
    _cb: &mut dyn TpEncCallbacks,
) -> i32 {
    let mut usac_config_bits = cc.raw_config_bits;

    // `(usacConfigBits + 7) / 8 > sizeof(rawConfig)`: both builds compile the test without the
    // overflow of `+ 7` (valid iff `usacConfigBits - 513 >= 0xFFFFFE00` unsigned, i.e. 1..=512).
    if usac_config_bits <= 0 || (usac_config_bits as i64 + 7) / 8 > cc.raw_config.len() as i64 {
        return TRANSPORTENC_UNSUPPORTED_FORMAT;
    }
    let padded = BitMem::new(128);
    padded.write(0, &cc.raw_config);
    let mut usac_conf = FdkBitStream::default();
    fdk_init_bit_stream(&mut usac_conf, &padded, 128, usac_config_bits as u32, BS_READER);

    while usac_config_bits > 0 {
        let tmp = fdk_read_bit(&mut usac_conf);
        fdk_write_bits(h_bs, tmp, 1);
        usac_config_bits -= 1;
    }
    fdk_sync_cache(h_bs);

    TRANSPORTENC_OK
}

/// `transportEnc_writeASC(asc, config, cb)`: writes an AudioSpecificConfig. Returns 0 on
/// success.
///
/// A channel mode of `MODE_INVALID` is replaced in `config` by the mode of
/// `transportEnc_GetChannelMode(noChannels)` (as in C, the caller's configuration changes).
pub fn transport_enc_write_asc(
    asc: &mut FdkBitStream,
    config: &mut CoderConfig,
    cb: &mut dyn TpEncCallbacks,
) -> i32 {
    let mut ext_flag: u32 = 0;
    let ep_config: i32 = 0;

    // Required for the PCE.
    let align_anchor = fdk_get_valid_bits(asc);

    // Extension Flag: Shall be 1 for aot = 17,19,20,21,22,23,39
    match config.aot {
        AOT_ER_AAC_LC | AOT_ER_AAC_LTP | AOT_ER_AAC_SCAL | AOT_ER_TWIN_VQ | AOT_ER_BSAC
        | AOT_ER_AAC_LD | AOT_ER_AAC_ELD | AOT_USAC => ext_flag = 1,
        _ => {}
    }

    if config.sbr_signaling == SIG_EXPLICIT_HIERARCHICAL && config.sbr_present != 0 {
        write_aot(asc, config.ext_aot);
    } else {
        write_aot(asc, config.aot);
    }

    // In case of USAC it is the output not the core sampling rate
    write_sample_rate(asc, config.sampling_rate, 4);

    // Try to guess a reasonable channel mode if not given
    if config.channel_mode == MODE_INVALID {
        config.channel_mode = transport_enc_get_channel_mode(config.no_channels);
        if config.channel_mode == MODE_INVALID {
            return -1;
        }
    }

    fdk_write_bits(
        asc,
        get_channel_config(config.channel_mode, config.channel_config_zero) as u32,
        4,
    );

    if config.sbr_signaling == SIG_EXPLICIT_HIERARCHICAL && config.sbr_present != 0 {
        write_sample_rate(asc, config.ext_sampling_rate, 4);
        write_aot(asc, config.aot);
    }

    match config.aot {
        AOT_AAC_MAIN | AOT_AAC_LC | AOT_AAC_SSR | AOT_AAC_LTP | AOT_AAC_SCAL | AOT_TWIN_VQ
        | AOT_ER_AAC_LC | AOT_ER_AAC_LTP | AOT_ER_AAC_SCAL | AOT_ER_TWIN_VQ | AOT_ER_BSAC
        | AOT_ER_AAC_LD => {
            let err =
                transport_enc_write_ga_specific_config(asc, config, ext_flag as i32, align_anchor);
            if err != 0 {
                return err;
            }
        }
        AOT_ER_AAC_ELD => {
            let err = transport_enc_write_eld_specific_config(asc, config, ep_config, cb);
            if err != 0 {
                return err;
            }
        }
        AOT_USAC => {
            let err = transport_enc_write_usac_specific_config(asc, ext_flag as i32, config, cb);
            if err != 0 {
                return err;
            }
        }
        _ => return -1,
    }

    match config.aot {
        AOT_ER_AAC_LC | AOT_ER_AAC_LTP | AOT_ER_AAC_SCAL | AOT_ER_TWIN_VQ | AOT_ER_BSAC
        | AOT_ER_AAC_LD | AOT_ER_CELP | AOT_ER_HVXC | AOT_ER_HILN | AOT_ER_PARA
        | AOT_ER_AAC_ELD => {
            fdk_write_bits(asc, 0, 2); // epconfig 0
        }
        _ => {}
    }

    // backward compatible explicit signaling of extension AOT
    if config.sbr_signaling == SIG_EXPLICIT_BW_COMPATIBLE {
        let mut _asc_ext_id = ASCEXT_UNKOWN;

        if config.sbr_present != 0 {
            _asc_ext_id = ASCEXT_SBR;
            fdk_write_bits(asc, ASCEXT_SBR as u32, 11);
            write_aot(asc, config.ext_aot);
            fdk_write_bits(asc, 1, 1); // sbrPresentFlag=1
            write_sample_rate(asc, config.ext_sampling_rate, 4);
            if config.ps_present != 0 {
                _asc_ext_id = ASCEXT_PS;
                fdk_write_bits(asc, ASCEXT_PS as u32, 11);
                fdk_write_bits(asc, 1, 1); // psPresentFlag=1
            }
        }
    }

    // Make sure all bits are sync'ed
    fdk_sync_cache(asc);

    0
}
