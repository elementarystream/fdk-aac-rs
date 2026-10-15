// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libPCMutils/include/pcmdmx_lib.h, libPCMutils/src/pcmdmx_lib.cpp.

//! Port of `pcmdmx_lib.h`: the PCM downmix module, which creates a mixdown, bypasses or extends
//! (upmixes) the PCM output signal depending on its settings, the input channel configuration and
//! the downmix metadata (DVB ancillary data, ETSI TS 101 154 or ISO/IEC 14496-3:2009/Amd.4, and
//! the matrix mixdown of the PCE).
//!
//! `HANDLE_PCM_DOWNMIX` becomes `Box<PcmDmxInstance>`; the functions take `Option<...>` where the
//! C++ code checks for `NULL`. The file-static helpers are public, so that they can be tested on
//! their own.
//!
//! Both reference builds compile the same code apart from the primitives (`fMult`, `fLdPow`) and
//! the `SATURATE_*` macros (`arm/scale_arm.h` and generic), which give the same values here.
//!
//! Memory: `pcmDmx_ApplyFrame()` addresses the PCM buffer through channel offset tables. With
//! consistent channel descriptions and the library's channel maps, every access is inside the
//! buffer of `pcmBufSize` samples. For other inputs the reference has no defined result; the
//! port reads 0 outside `pcm_buf` and drops writes outside it, and ignores writes outside the
//! channel type and index arrays (PORTING.md, "Deviations from the reference").

use fdk_aac_rs_fdk::audio::{
    lib_version, lib_version_string, AudioChannelType, LibInfo, ACT_BACK, ACT_FRONT, ACT_FRONT_TOP,
    ACT_LFE, ACT_NONE, ACT_SIDE, CAPF_DMX_6_CH, CAPF_DMX_8_CH, CAPF_DMX_ARIB, CAPF_DMX_BLIND,
    CAPF_DMX_CH_EXP, CAPF_DMX_DVB, CAPF_DMX_PCE, FDK_MODULE_LAST, FDK_NONE, FDK_PCMDMX,
};
use fdk_aac_rs_fdk::bitbuffer::BitMem;
use fdk_aac_rs_fdk::bitstream::{
    fdk_get_valid_bits, fdk_init_bit_stream, fdk_push_for, fdk_read_bit, fdk_read_bits,
    FdkBitStream, BS_READER,
};
use fdk_aac_rs_fdk::common_fix::*;
use fdk_aac_rs_fdk::core::fdk_tools_get_lib_info;
use fdk_aac_rs_fdk::fdk_assert;
use fdk_aac_rs_fdk::fixpoint_math::f_ld_pow;
use fdk_aac_rs_fdk::scale::{saturate_right_shift, saturate_shift};
use fdk_aac_rs_fdk::syslib_channel_map_descr::{
    fdk_ch_map_descr_get_map_value, fdk_ch_map_descr_is_valid, FdkChannelMapDescr,
};

use crate::pcm_utils::{
    PCMUTIL_LIB_BUILD_DATE, PCMUTIL_LIB_BUILD_TIME, PCMUTIL_LIB_VL0, PCMUTIL_LIB_VL1,
    PCMUTIL_LIB_VL2,
};

// ---------------------------------------------------------------------------------------------
// pcmdmx_lib.h
// ---------------------------------------------------------------------------------------------

/// `PCMDMX_ERROR`
pub type PcmdmxError = i32;
/// `PCMDMX_OK`: no error happened.
pub const PCMDMX_OK: PcmdmxError = 0x0;
/// `PCMDMX_UNSUPPORTED`: the requested feature/service is unavailable.
pub const PCMDMX_UNSUPPORTED: PcmdmxError = 0x1;
/// `pcm_dmx_fatal_error_start`
pub const pcm_dmx_fatal_error_start: PcmdmxError = 2;
/// `PCMDMX_OUT_OF_MEMORY`: not enough memory to set up an instance of the module.
pub const PCMDMX_OUT_OF_MEMORY: PcmdmxError = 3;
/// `pcm_dmx_fatal_error_end`
pub const pcm_dmx_fatal_error_end: PcmdmxError = 4;
/// `PCMDMX_INVALID_HANDLE`: the given instance handle is not valid.
pub const PCMDMX_INVALID_HANDLE: PcmdmxError = 5;
/// `PCMDMX_INVALID_ARGUMENT`: one of the parameters handed over is invalid.
pub const PCMDMX_INVALID_ARGUMENT: PcmdmxError = 6;
/// `PCMDMX_INVALID_CH_CONFIG`: the given channel configuration is not supported.
pub const PCMDMX_INVALID_CH_CONFIG: PcmdmxError = 7;
/// `PCMDMX_INVALID_MODE`: the set configuration/mode is not applicable.
pub const PCMDMX_INVALID_MODE: PcmdmxError = 8;
/// `PCMDMX_UNKNOWN_PARAM`: the handed parameter is not known/supported.
pub const PCMDMX_UNKNOWN_PARAM: PcmdmxError = 9;
/// `PCMDMX_UNABLE_TO_SET_PARAM`: unable to set the specific parameter.
pub const PCMDMX_UNABLE_TO_SET_PARAM: PcmdmxError = 10;
/// `PCMDMX_CORRUPT_ANC_DATA`: the read ancillary data was corrupt.
pub const PCMDMX_CORRUPT_ANC_DATA: PcmdmxError = 11;
/// `PCMDMX_OUTPUT_BUFFER_TOO_SMALL`: the size of the PCM output buffer is too small.
pub const PCMDMX_OUTPUT_BUFFER_TOO_SMALL: PcmdmxError = 12;

/// `PCMDMX_IS_FATAL_ERROR(err)`
#[inline]
pub const fn pcmdmx_is_fatal_error(err: PcmdmxError) -> i32 {
    if err >= pcm_dmx_fatal_error_start && err <= pcm_dmx_fatal_error_end {
        1
    } else {
        0
    }
}

/// `PCMDMX_PARAM`
pub type PcmdmxParam = i32;
/// `DMX_PROFILE_SETTING`
pub const DMX_PROFILE_SETTING: PcmdmxParam = 0x01;
/// `DMX_BS_DATA_EXPIRY_FRAME`
pub const DMX_BS_DATA_EXPIRY_FRAME: PcmdmxParam = 0x10;
/// `DMX_BS_DATA_DELAY`
pub const DMX_BS_DATA_DELAY: PcmdmxParam = 0x11;
/// `MIN_NUMBER_OF_OUTPUT_CHANNELS`
pub const MIN_NUMBER_OF_OUTPUT_CHANNELS: PcmdmxParam = 0x20;
/// `MAX_NUMBER_OF_OUTPUT_CHANNELS`
pub const MAX_NUMBER_OF_OUTPUT_CHANNELS: PcmdmxParam = 0x21;
/// `DMX_DUAL_CHANNEL_MODE`
pub const DMX_DUAL_CHANNEL_MODE: PcmdmxParam = 0x30;
/// `DMX_PSEUDO_SURROUND_MODE`
pub const DMX_PSEUDO_SURROUND_MODE: PcmdmxParam = 0x31;

/// `DMX_PROFILE_TYPE`
pub type DmxProfileType = i32;
/// `DMX_PRFL_STANDARD`
pub const DMX_PRFL_STANDARD: DmxProfileType = 0x0;
/// `DMX_PRFL_MATRIX_MIX`
pub const DMX_PRFL_MATRIX_MIX: DmxProfileType = 0x1;
/// `DMX_PRFL_FORCE_MATRIX_MIX`
pub const DMX_PRFL_FORCE_MATRIX_MIX: DmxProfileType = 0x2;
/// `DMX_PRFL_ARIB_JAPAN`
pub const DMX_PRFL_ARIB_JAPAN: DmxProfileType = 0x3;

/// `PSEUDO_SURROUND_MODE`
pub type PseudoSurroundMode = i32;
/// `NEVER_DO_PS_DMX`
pub const NEVER_DO_PS_DMX: PseudoSurroundMode = -1;
/// `AUTO_PS_DMX`
pub const AUTO_PS_DMX: PseudoSurroundMode = 0;
/// `FORCE_PS_DMX`
pub const FORCE_PS_DMX: PseudoSurroundMode = 1;

/// `DUAL_CHANNEL_MODE`
pub type DualChannelMode = i32;
/// `STEREO_MODE`
pub const STEREO_MODE: DualChannelMode = 0x0;
/// `CH1_MODE`
pub const CH1_MODE: DualChannelMode = 0x1;
/// `CH2_MODE`
pub const CH2_MODE: DualChannelMode = 0x2;
/// `MIXED_MODE`
pub const MIXED_MODE: DualChannelMode = 0x3;

/// `DMX_PCM`
pub type DmxPcm = FixpDbl;
/// `DMX_PCM_BITS`
pub const DMX_PCM_BITS: i32 = DFRACT_BITS;

/// `PCMDMX_RESET_PARAMS`
pub const PCMDMX_RESET_PARAMS: u32 = 1;
/// `PCMDMX_RESET_BS_DATA`
pub const PCMDMX_RESET_BS_DATA: u32 = 2;
/// `PCMDMX_RESET_FULL`
pub const PCMDMX_RESET_FULL: u32 = PCMDMX_RESET_PARAMS | PCMDMX_RESET_BS_DATA;

// ---------------------------------------------------------------------------------------------
// pcmdmx_lib.cpp: definitions
// ---------------------------------------------------------------------------------------------

/// `PCMDMX_LIB_TITLE`
pub const PCMDMX_LIB_TITLE: &str = "PCM Downmix Lib";

/// `FIXP_DMX`
pub type FixpDmx = FixpSgl;

/// `FL2FXCONST_DMX(x)` of a `float` literal.
const fn fl2fxconst_dmx(x: f32) -> FixpDmx {
    fl2fxconst_sgl(x as f64)
}

// Fixed and unique channel group indices.
pub const CH_GROUP_FRONT: usize = 0;
pub const CH_GROUP_SIDE: usize = 1;
pub const CH_GROUP_REAR: usize = 2;
pub const CH_GROUP_LFE: usize = 3;

// Fixed and unique channel plain indices.
pub const CH_PLAIN_NORMAL: i32 = 0;
pub const CH_PLAIN_TOP: i32 = 1;
pub const CH_PLAIN_BOTTOM: i32 = 2;

// The ordering of the following fixed channel labels has to be in MPEG-4 style. From the center
// to the back with left and right channel interleaved (starting with left). The last channel
// label index has to be smaller than ( 8 ).
pub const CENTER_FRONT_CHANNEL: usize = 0; // C
pub const LEFT_FRONT_CHANNEL: usize = 1; // L
pub const RIGHT_FRONT_CHANNEL: usize = 2; // R
pub const LEFT_REAR_CHANNEL: usize = 3; // Lr (aka left back channel) or center back channel
pub const RIGHT_REAR_CHANNEL: usize = 4; // Rr (aka right back channel)
pub const LOW_FREQUENCY_CHANNEL: usize = 5; // Lf
pub const LEFT_MULTIPRPS_CHANNEL: usize = 6; // Left multipurpose channel
pub const RIGHT_MULTIPRPS_CHANNEL: usize = 7; // Right multipurpose channel

// 22.2 channel specific fixed channel lables:
pub const LEFT_SIDE_CHANNEL: usize = 8; // Lss
pub const RIGHT_SIDE_CHANNEL: usize = 9; // Rss
pub const CENTER_REAR_CHANNEL: usize = 10; // Cs
pub const CENTER_FRONT_CHANNEL_TOP: usize = 11; // Cv
pub const LEFT_FRONT_CHANNEL_TOP: usize = 12; // Lv
pub const RIGHT_FRONT_CHANNEL_TOP: usize = 13; // Rv
pub const LEFT_SIDE_CHANNEL_TOP: usize = 14; // Lvss
pub const RIGHT_SIDE_CHANNEL_TOP: usize = 15; // Rvss
pub const CENTER_SIDE_CHANNEL_TOP: usize = 16; // Ts
pub const LEFT_REAR_CHANNEL_TOP: usize = 17; // Lvr
pub const RIGHT_REAR_CHANNEL_TOP: usize = 18; // Rvr
pub const CENTER_REAR_CHANNEL_TOP: usize = 19; // Cvr
pub const CENTER_FRONT_CHANNEL_BOTTOM: usize = 20; // Cb
pub const LEFT_FRONT_CHANNEL_BOTTOM: usize = 21; // Lb
pub const RIGHT_FRONT_CHANNEL_BOTTOM: usize = 22; // Rb
pub const LOW_FREQUENCY_CHANNEL_2: usize = 23; // LFE2

// More constants
pub const ONE_CHANNEL: i32 = 1;
pub const TWO_CHANNEL: i32 = 2;
pub const SIX_CHANNEL: i32 = 6;
pub const EIGHT_CHANNEL: i32 = 8;
pub const TWENTY_FOUR_CHANNEL: i32 = 24;

pub const PCMDMX_THRESHOLD_MAP_HEAT_1: u32 = 0; // Store only exact matches
pub const PCMDMX_THRESHOLD_MAP_HEAT_2: u32 = 20;
pub const PCMDMX_THRESHOLD_MAP_HEAT_3: u32 = 256; // Do not assign normal channels to LFE

pub const SP_Z_NRM: i8 = 0;
pub const SP_Z_TOP: i8 = 2;
pub const SP_Z_BOT: i8 = -2;
pub const SP_Z_LFE: i8 = -18;
pub const SP_Z_MUL: i8 = 8; // Should be smaller than SP_Z_LFE

/// `PCM_DMX_SPEAKER_POSITION`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PcmDmxSpeakerPosition {
    /// horizontal position: center (0), left (-), right (+)
    pub x: i8,
    /// deepth position: front, side, back, position
    pub y: i8,
    /// heigth positions: normal, top, bottom, lfe
    pub z: i8,
}

/// `spkrSlotPos[]` as `x, y, z` triples. Generated with scripts/dump_table.py.
/// CAUTION: The maximum x-value should be less or equal to PCMDMX_SPKR_POS_X_MAX_WIDTH.
pub static SPKR_SLOT_POS: [i8; 24] = [
    0, 0, 0, //
    -2, 0, 0, //
    2, 0, 0, //
    -3, 4, 0, //
    3, 4, 0, //
    0, 0, -18, //
    -2, 2, 8, //
    2, 2, 8, //
];

/// `spkrSlotPos[i]`
#[inline]
fn spkr_slot_pos(i: usize) -> PcmDmxSpeakerPosition {
    PcmDmxSpeakerPosition {
        x: SPKR_SLOT_POS[3 * i],
        y: SPKR_SLOT_POS[3 * i + 1],
        z: SPKR_SLOT_POS[3 * i + 2],
    }
}

/// `PCM_DMX_CHANNEL_MODE`: `CH_MODE_<numFrontCh>_<numSideCh>_<numBackCh>_<numLfCh>`
pub type PcmDmxChannelMode = i32;
pub const CH_MODE_UNDEFINED: PcmDmxChannelMode = 0x0000;
// 1 channel
pub const CH_MODE_1_0_0_0: PcmDmxChannelMode = 0x0001; // chCfg 1
                                                       // 2 channels
pub const CH_MODE_2_0_0_0: PcmDmxChannelMode = 0x0002; // chCfg 2
                                                       // 3 channels
pub const CH_MODE_3_0_0_0: PcmDmxChannelMode = 0x0003; // chCfg 3
pub const CH_MODE_2_0_1_0: PcmDmxChannelMode = 0x0102;
pub const CH_MODE_2_0_0_1: PcmDmxChannelMode = 0x1002;
// 4 channels
pub const CH_MODE_3_0_1_0: PcmDmxChannelMode = 0x0103; // chCfg 4
pub const CH_MODE_2_0_2_0: PcmDmxChannelMode = 0x0202;
pub const CH_MODE_2_0_1_1: PcmDmxChannelMode = 0x1102;
pub const CH_MODE_4_0_0_0: PcmDmxChannelMode = 0x0004;
// 5 channels
pub const CH_MODE_3_0_2_0: PcmDmxChannelMode = 0x0203; // chCfg 5
pub const CH_MODE_2_0_2_1: PcmDmxChannelMode = 0x1202;
pub const CH_MODE_3_0_1_1: PcmDmxChannelMode = 0x1103;
pub const CH_MODE_3_2_0_0: PcmDmxChannelMode = 0x0023;
pub const CH_MODE_5_0_0_0: PcmDmxChannelMode = 0x0005;
// 6 channels
pub const CH_MODE_3_0_2_1: PcmDmxChannelMode = 0x1203; // chCfg 6
pub const CH_MODE_3_2_0_1: PcmDmxChannelMode = 0x1023;
pub const CH_MODE_3_2_1_0: PcmDmxChannelMode = 0x0123;
pub const CH_MODE_5_0_1_0: PcmDmxChannelMode = 0x0105;
pub const CH_MODE_6_0_0_0: PcmDmxChannelMode = 0x0006;
// 7 channels
pub const CH_MODE_2_2_2_1: PcmDmxChannelMode = 0x1222;
pub const CH_MODE_3_0_3_1: PcmDmxChannelMode = 0x1303; // chCfg 11
pub const CH_MODE_3_2_1_1: PcmDmxChannelMode = 0x1123;
pub const CH_MODE_3_2_2_0: PcmDmxChannelMode = 0x0223;
pub const CH_MODE_3_0_2_2: PcmDmxChannelMode = 0x2203;
pub const CH_MODE_5_0_2_0: PcmDmxChannelMode = 0x0205;
pub const CH_MODE_5_0_1_1: PcmDmxChannelMode = 0x1105;
pub const CH_MODE_7_0_0_0: PcmDmxChannelMode = 0x0007;
// 8 channels
pub const CH_MODE_3_2_2_1: PcmDmxChannelMode = 0x1223;
pub const CH_MODE_3_0_4_1: PcmDmxChannelMode = 0x1403; // chCfg 12
pub const CH_MODE_5_0_2_1: PcmDmxChannelMode = 0x1205; // chCfg 7 + 14
pub const CH_MODE_5_2_1_0: PcmDmxChannelMode = 0x0125;
pub const CH_MODE_3_2_1_2: PcmDmxChannelMode = 0x2123;
pub const CH_MODE_2_2_2_2: PcmDmxChannelMode = 0x2222;
pub const CH_MODE_3_0_3_2: PcmDmxChannelMode = 0x2303;
pub const CH_MODE_8_0_0_0: PcmDmxChannelMode = 0x0008;

/// `outChModeTable[]`: the channel configurations linked to the number of output channels given
/// by the user. Generated with scripts/dump_table.py.
pub static OUT_CH_MODE_TABLE: [PcmDmxChannelMode; 9] = [
    0x00000000, 0x00000001, 0x00000002, 0x00000003, 0x00000103, 0x00000203, 0x00001203, 0x00001303,
    0x00001403,
];

/// `abMixLvlValueTab[]`. Generated with scripts/dump_table.py.
pub static AB_MIX_LVL_VALUE_TAB: [FixpDmx; 8] =
    [16384, 27558, 23167, 19530, 16384, 13828, 11633, 0];

/// `lfeMixLvlValueTab[]`. Generated with scripts/dump_table.py.
pub static LFE_MIX_LVL_VALUE_TAB: [FixpDmx; 16] = [
    25903, 16384, 27509, 23151, 19481, 16384, 27558, 23167, 19530, 16384, 10355, 5833, 3277, 1049,
    328, 0,
];

/// `mpegMixDownIdx2Coef[]`. Generated with scripts/dump_table.py.
pub static MPEG_MIX_DOWN_IDX2_COEF: [FixpDmx; 4] = [23170, 16384, 11585, 0];

/// `mpegMixDownIdx2PreFact[3][4]`. Generated with scripts/dump_table.py.
pub static MPEG_MIX_DOWN_IDX2_PRE_FACT: [[FixpDmx; 4]; 3] =
    [[13573, 14847, 15902, 19195], [10498, 12104, 13573, 19195], [7423, 8192, 8839, 10923]];

pub const TYPE_NONE: u32 = 0x00;
pub const TYPE_PCE_DATA: u32 = 0x01;
pub const TYPE_DSE_CLEV_DATA: u32 = 0x02;
pub const TYPE_DSE_SLEV_DATA: u32 = 0x04;
pub const TYPE_DSE_DMIX_AB_DATA: u32 = 0x08;
pub const TYPE_DSE_DMIX_LFE_DATA: u32 = 0x10;
pub const TYPE_DSE_DMX_GAIN_DATA: u32 = 0x20;
pub const TYPE_DSE_DMX_CGL_DATA: u32 = 0x40;
pub const TYPE_DSE_DATA: u32 = 0x7E;

/// `DMX_BS_META_DATA`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DmxBsMetaData {
    pub type_flags: u32,
    // From DSE
    pub c_lev_idx: u8,
    pub s_lev_idx: u8,
    pub dmix_idx_a: u8,
    pub dmix_idx_b: u8,
    pub dmix_idx_lfe: u8,
    pub dmx_gain_idx2: u8,
    pub dmx_gain_idx5: u8,
    // From PCE
    pub matrix_mixdown_idx: u8,
    // Attributes:
    /// If set to 1 the signal is pseudo surround compatible. The value 0 tells that it is not.
    /// If the value is -1 the information is not available.
    pub pseudo_surround: i8,
    /// Counter to monitor the life time of a meta data set.
    pub expiry_count: u32,
}

/// `dfltMetaData`: default metadata (`{0, 2, 2, 2, 2, 15, 0, 0, 0, -1, 0}`).
pub const DFLT_META_DATA: DmxBsMetaData = DmxBsMetaData {
    type_flags: 0,
    c_lev_idx: 2,
    s_lev_idx: 2,
    dmix_idx_a: 2,
    dmix_idx_b: 2,
    dmix_idx_lfe: 15,
    dmx_gain_idx2: 0,
    dmx_gain_idx5: 0,
    matrix_mixdown_idx: 0,
    pseudo_surround: -1,
    expiry_count: 0,
};

/// `PCM_DMX_USER_PARAMS`: dynamic (user) params. See the definition of `PCMDMX_PARAM` for details
/// on the specific fields.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PcmDmxUserParams {
    /// Linked to DMX_PRFL_STANDARD
    pub dmx_profile: DmxProfileType,
    /// Linked to DMX_BS_DATA_EXPIRY_FRAME
    pub expiry_frame: u32,
    /// Linked to DMX_DUAL_CHANNEL_MODE
    pub dual_channel_mode: DualChannelMode,
    /// Linked to DMX_PSEUDO_SURROUND_MODE
    pub pseudo_surr_mode: PseudoSurroundMode,
    /// Linked to MIN_NUMBER_OF_OUTPUT_CHANNELS
    pub num_out_channels_min: i16,
    /// Linked to MAX_NUMBER_OF_OUTPUT_CHANNELS
    pub num_out_channels_max: i16,
    /// Linked to DMX_BS_DATA_DELAY
    pub frame_delay: u8,
}

/// `struct PCM_DMX_INSTANCE`: the module's main data structure. `Default` is the zeroed memory
/// of `GetPcmDmxInstance()` (`FDKcalloc`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PcmDmxInstance {
    // Metadata
    pub bs_meta_data: [DmxBsMetaData; 1 + 1],
    pub user_params: PcmDmxUserParams,
    /// Flag to en-/disable modules processing. The max channel limiting is done independently.
    pub apply_processing: u8,
}

/// `HANDLE_PCM_DOWNMIX`
pub type HandlePcmDownmix = Box<PcmDmxInstance>;

// ---------------------------------------------------------------------------------------------
// Speaker positions and channel mapping
// ---------------------------------------------------------------------------------------------

/// `getSpeakerDistance(posA, posB)` (file-static): the squared distance; the differences are
/// truncated to `SCHAR`.
pub fn get_speaker_distance(pos_a: PcmDmxSpeakerPosition, pos_b: PcmDmxSpeakerPosition) -> u32 {
    let dx = pos_a.x.wrapping_sub(pos_b.x) as i32;
    let dy = pos_a.y.wrapping_sub(pos_b.y) as i32;
    let dz = pos_a.z.wrapping_sub(pos_b.z) as i32;

    ((dx * dx) + (dy * dy) + (dz * dz)) as u32
}

const PCMDMX_SPKR_POS_X_MAX_WIDTH: i32 = 3;
const PCMDMX_SPKR_POS_Y_SPREAD: i32 = 2;
const PCMDMX_SPKR_POS_Z_SPREAD: i32 = 2;

/// `getSpeakerPos(chType, chIndex, numChInGrp)` (file-static). The `SCHAR` casts and stores of
/// the C++ code truncate.
pub fn get_speaker_pos(
    ch_type: AudioChannelType,
    mut ch_index: u8,
    num_ch_in_grp: u8,
) -> PcmDmxSpeakerPosition {
    let mut spkr_pos = PcmDmxSpeakerPosition::default();
    let ch_grp = ch_type & 0x0F;
    let f_has_center = (num_ch_in_grp & 0x1) as u32;
    let ch_grp_width = (num_ch_in_grp >> 1) as u32;
    let mut f_is_center = 0u32;
    let f_is_lfe = if ch_type == ACT_LFE { 1u32 } else { 0 };
    let mut offset: i32 = 0;

    // Reachable with consistent input: getChannelMode() does not validate the indices of
    // ACT_NONE channels.
    fdk_assert!(ch_index < num_ch_in_grp);

    if ch_grp == ACT_FRONT && f_has_center != 0 {
        if ch_index == 0 {
            f_is_center = 1;
        }
        ch_index = f_max(0, ch_index as i32 - 1) as u8;
    } else if f_has_center != 0 && ch_index as i32 == num_ch_in_grp as i32 - 1 {
        f_is_center = 1;
    }
    // now all even indices are left (-)
    if f_is_center == 0 {
        offset = (ch_index >> 1) as i32;
        if ch_grp > ACT_FRONT && ch_type != ACT_SIDE && f_is_lfe == 0 {
            // the higher the index the lower the distance to the center position
            // (unsigned arithmetic in C++)
            offset = ch_grp_width.wrapping_sub(f_has_center).wrapping_sub(offset as u32) as i32;
        }
        if ch_index & 0x1 == 0 {
            // even
            offset = offset.wrapping_add(1).wrapping_neg();
        } else {
            offset = offset.wrapping_add(1);
        }
    }
    // apply the offset
    if ch_type == ACT_SIDE {
        spkr_pos.x =
            if offset < 0 { -PCMDMX_SPKR_POS_X_MAX_WIDTH } else { PCMDMX_SPKR_POS_X_MAX_WIDTH }
                as i8;
        // (SCHAR)fAbs(offset)
        spkr_pos.y = (PCMDMX_SPKR_POS_Y_SPREAD + (offset.wrapping_abs() as i8) as i32 - 1) as i8;
        spkr_pos.z = 0;
    } else {
        let spread: u32 = if ch_grp_width == 1 && f_is_lfe == 0 {
            (PCMDMX_SPKR_POS_X_MAX_WIDTH - 1) as u32
        } else {
            1
        };
        spkr_pos.x = ((offset as i8) as i32 * (spread as i8) as i32) as i8;
        if f_is_lfe != 0 {
            spkr_pos.y = 0;
            spkr_pos.z = SP_Z_LFE;
        } else {
            spkr_pos.y = ((f_max((ch_grp as i8) as i32 - 1, 0) as i8) as i32
                * PCMDMX_SPKR_POS_Y_SPREAD) as i8;
            spkr_pos.z = (ch_type as i8) >> 4;
            if spkr_pos.z == 2 {
                // ACT_BOTTOM
                spkr_pos.z = -1;
            }
            spkr_pos.z = (spkr_pos.z as i32 * PCMDMX_SPKR_POS_Z_SPREAD) as i8;
        }
    }
    spkr_pos
}

/// `getChMode4Plain(plainIndex, totChMode, chCfg)` (file-static): the channel mode of a given
/// horizontal channel plain (normal, top, bottom) for a given channel configuration.
pub fn get_ch_mode4_plain(
    plain_index: i32,
    tot_ch_mode: PcmDmxChannelMode,
    ch_cfg: i32,
) -> PcmDmxChannelMode {
    let mut plain_ch_mode = tot_ch_mode;

    if tot_ch_mode == CH_MODE_5_0_2_1 && ch_cfg == 14 {
        plain_ch_mode = match plain_index {
            CH_PLAIN_BOTTOM => 0x0000,
            CH_PLAIN_TOP => CH_MODE_2_0_0_0,
            _ => CH_MODE_3_0_2_1,
        };
    }

    plain_ch_mode
}

/// `validateIndices(numChannels, numChannelsPlaneAndGrp, aChType, channelType, channelIndices)`
/// (file-static): 1 if the channel indices of the channels of type `a_ch_type` are consecutive
/// and unique, 0 otherwise.
pub fn validate_indices(
    num_channels: u32,
    num_channels_plane_and_grp: u32,
    a_ch_type: AudioChannelType,
    channel_type: &[AudioChannelType; 8],
    channel_indices: &[u8; 8],
) -> u32 {
    for req_value in 0..num_channels_plane_and_grp {
        let mut found = false;
        for i in 0..(num_channels as usize).min(8) {
            if channel_type[i] == a_ch_type && channel_indices[i] as u32 == req_value {
                if found {
                    return 0; // Found channel index a second time
                } else {
                    found = true; // Found channel index
                }
            }
        }
        if !found {
            return 0; // Did not find channel index
        }
    }
    1 // Successfully validated channel indices
}

/// The flat index of `numCh[chType >> 4][chGrp]` (`UCHAR numCh[3][4]`), `None` outside the
/// array. Inside the array, the C++ code addresses `numCh` as one array of 12 entries (channel
/// types such as `0x05` address the next plane); outside it accesses the neighbouring stack
/// memory, which the port does not reproduce (the channel types of the library never get there).
#[inline]
fn num_ch_index(ch_type: AudioChannelType) -> Option<usize> {
    let ch_grp = f_max((ch_type & 0x0F) - 1, 0);
    let idx = (ch_type >> 4) as i64 * 4 + ch_grp as i64;
    if (0..12).contains(&idx) {
        Some(idx as usize)
    } else {
        None
    }
}

/// `getChannelMode(numChannels, channelType, channelIndices, offsetTable, chMode)`
/// (file-static): evaluates a given channel configuration and extracts a packed channel mode. In
/// addition the function generates a channel offset table for the mapping to the internal
/// representation. Invalid channel indices are replaced by consecutive ones (and
/// `PCMDMX_INVALID_CH_CONFIG` is returned). `num_channels` is at most 8.
pub fn get_channel_mode(
    num_channels: u32,
    channel_type: &[AudioChannelType; 8],
    channel_indices: &mut [u8; 8],
    offset_table: &mut [u8; 8],
    ch_mode: &mut PcmDmxChannelMode,
) -> PcmdmxError {
    let mut num_ch = [0u8; 3 * 4];
    let mut mapped = [0u8; 8];
    let mut spkr_pos = [PcmDmxSpeakerPosition::default(); 8];
    let mut err = PCMDMX_OK;
    let mut num_mapped_in_chs = 0u32;
    let stop_slot = LOW_FREQUENCY_CHANNEL;
    let num_channels_us = (num_channels as usize).min(8);

    // For details see ISO/IEC 13818-7:2005(E), 8.5.3 Channel configuration
    // Init output
    offset_table.fill(255);
    *ch_mode = CH_MODE_UNDEFINED;

    // Determine how many channels are assigned to each channels each group:
    for ch in 0..num_channels_us {
        // Assign all undefined channels (ACT_NONE) to front channels.
        if let Some(i) = num_ch_index(channel_type[ch]) {
            num_ch[i] = num_ch[i].wrapping_add(1);
        }
    }

    // Sanity check on the indices
    for ch_grp in 0..4usize {
        for plane in 0..3usize {
            if num_ch[plane * 4 + ch_grp] == 0 {
                continue;
            }
            let a_ch_type = ((plane as i32) << 4) | ((ch_grp as i32 + 1) & 0xF);
            if validate_indices(
                num_channels,
                num_ch[plane * 4 + ch_grp] as u32,
                a_ch_type,
                channel_type,
                channel_indices,
            ) == 0
            {
                let mut idx_cnt = 0u32;
                for ch in 0..num_channels_us {
                    if channel_type[ch] == a_ch_type {
                        channel_indices[ch] = idx_cnt as u8;
                        idx_cnt += 1;
                    }
                }
                err = PCMDMX_INVALID_CH_CONFIG;
            }
        }
    }

    // Mapping HEAT 1:
    //   Determine the speaker position of each input channel and map it to a internal slot if it
    //   matches exactly (with zero distance).
    for ch in 0..num_channels_us {
        let mut map_dist = u32::MAX;
        let mut map_pos = u32::MAX as usize;
        let n = num_ch_index(channel_type[ch]).map_or(0, |i| num_ch[i]);

        spkr_pos[ch] = get_speaker_pos(channel_type[ch], channel_indices[ch], n);

        for map_ch in 0..=stop_slot {
            if offset_table[map_ch] == 255 {
                let dist = get_speaker_distance(spkr_pos[ch], spkr_slot_pos(map_ch));
                if dist < map_dist {
                    map_pos = map_ch;
                    map_dist = dist;
                }
            }
        }
        if map_dist <= PCMDMX_THRESHOLD_MAP_HEAT_1 {
            offset_table[map_pos] = ch as u8;
            mapped[ch] = 1;
            num_mapped_in_chs += 1;
        }
    }

    // Mapping HEAT 2:
    //   Go through the unmapped input channels and assign them to the internal slots that
    //   matches best (least distance). But assign center channels to center slots only.
    let start_slot: usize = if (num_ch[CH_PLAIN_NORMAL as usize * 4 + CH_GROUP_FRONT] & 0x1) != 0
        || num_channels >= 8
    {
        0
    } else {
        1
    };
    for ch in 0..num_channels_us {
        if mapped[ch] == 0 {
            let mut map_dist = u32::MAX;
            let mut map_pos = u32::MAX as usize;

            for map_ch in start_slot..=stop_slot {
                if offset_table[map_ch] == 255 {
                    let dist = get_speaker_distance(spkr_pos[ch], spkr_slot_pos(map_ch));
                    if dist < map_dist {
                        map_pos = map_ch;
                        map_dist = dist;
                    }
                }
            }
            if map_pos <= stop_slot
                && map_dist < PCMDMX_THRESHOLD_MAP_HEAT_2
                && ((spkr_pos[ch].x != 0 && spkr_slot_pos(map_pos).x != 0) // XOR
                    || (spkr_pos[ch].x == 0 && spkr_slot_pos(map_pos).x == 0))
            {
                // Assign center channels to center slots only.
                offset_table[map_pos] = ch as u8;
                mapped[ch] = 1;
                num_mapped_in_chs += 1;
            }
        }
    }

    // Mapping HEAT 3:
    //   Assign the rest by searching for the nearest input channel for each internal slot.
    let mut ch = start_slot;
    while ch < 8 && num_mapped_in_chs < num_channels {
        if offset_table[ch] == 255 {
            let mut map_dist = u32::MAX;
            let mut map_pos = u32::MAX as usize;

            for map_ch in 0..num_channels_us {
                if mapped[map_ch] == 0 {
                    let dist = get_speaker_distance(spkr_pos[map_ch], spkr_slot_pos(ch));
                    if dist < map_dist {
                        map_pos = map_ch;
                        map_dist = dist;
                    }
                }
            }
            if map_dist < PCMDMX_THRESHOLD_MAP_HEAT_3 {
                offset_table[ch] = map_pos as u8;
                mapped[map_pos] = 1;
                num_mapped_in_chs += 1;
                if spkr_pos[map_pos].x == 0 && spkr_slot_pos(ch).x != 0 && num_channels < 8 {
                    // Skip the paired slot if we assigned a center channel.
                    ch += 1;
                }
            }
        }
        ch += 1;
    }

    // Finaly compose the channel mode
    for ch in 0..4usize {
        let mut num_ch_in_grp = 0i32;
        for plane in 0..3usize {
            num_ch_in_grp += num_ch[plane * 4 + ch] as i32;
        }
        *ch_mode |= num_ch_in_grp << (ch * 4);
    }

    err
}

/// `channelType[i] = v`, ignored outside the array.
#[inline]
fn set_at<T: Copy>(a: &mut [T; 8], i: usize, v: T) {
    if let Some(e) = a.get_mut(i) {
        *e = v;
    }
}

/// `getChannelDescription(chMode, mapDescr, channelType, channelIndices, offsetTable)`
/// (file-static): generates a channel offset table and the complete channel description for a
/// given (packed) channel mode. Inverse of [`get_channel_mode`] but does not support weird
/// channel configurations.
pub fn get_channel_description(
    ch_mode: PcmDmxChannelMode,
    map_descr: &FdkChannelMapDescr,
    channel_type: &mut [AudioChannelType; 8],
    channel_indices: &mut [u8; 8],
    offset_table: &mut [u8; 8],
) {
    let num_plains = 1;
    let mut num_total_channels = 0;
    let mut ch: i32 = 0;

    // Init output arrays
    channel_type.fill(ACT_NONE);
    channel_indices.fill(0);
    offset_table.fill(255);

    // Summerize to get the total number of channels
    for grp_idx in 0..4 {
        num_total_channels += (ch_mode >> (grp_idx * 4)) & 0xF;
    }

    // Get the appropriate channel map
    let ch_cfg: i32 = match ch_mode {
        CH_MODE_1_0_0_0 | CH_MODE_2_0_0_0 | CH_MODE_3_0_0_0 | CH_MODE_3_0_1_0 | CH_MODE_3_0_2_0
        | CH_MODE_3_0_2_1 => num_total_channels,
        CH_MODE_3_0_3_1 => 11,
        CH_MODE_3_0_4_1 => 12,
        CH_MODE_5_0_2_1 => 7,
        // fallback
        _ => 0,
    };

    // Compose channel offset table
    for plain_idx in 0..num_plains {
        let plain_ch_mode = get_ch_mode4_plain(plain_idx, ch_mode, ch_cfg);

        // Extract the number of channels per group
        let num_ch_in_grp: [i32; 4] = [
            plain_ch_mode & 0xF,
            (plain_ch_mode >> 4) & 0xF,
            (plain_ch_mode >> 8) & 0xF,
            (plain_ch_mode >> 12) & 0xF,
        ];

        // Non-symmetric channels
        if (num_ch_in_grp[CH_GROUP_FRONT] & 0x1) != 0 && plain_idx == CH_PLAIN_NORMAL {
            // Odd number of front channels -> we have a center channel. In MPEG-4 the center has
            // the index 0.
            let mapped_idx =
                fdk_ch_map_descr_get_map_value(map_descr, ch as u8, ch_cfg as u32) as usize;
            offset_table[CENTER_FRONT_CHANNEL] = mapped_idx as u8;
            set_at(channel_type, mapped_idx, ACT_FRONT);
            set_at(channel_indices, mapped_idx, 0);
            ch += 1;
        }

        for grp_idx in 0..4usize {
            let mut type_: AudioChannelType = ACT_NONE;
            let mut ch_map_pos: usize = 0;
            let mut max_channels: i32 = 0;
            let mut ch_idx: i32 = 0; // Index of channel within the specific group

            match grp_idx {
                CH_GROUP_FRONT => {
                    type_ = (plain_idx << 4) | ACT_FRONT;
                    ch_map_pos = LEFT_FRONT_CHANNEL;
                    ch_idx = num_ch_in_grp[grp_idx] & 0x1;
                    max_channels = 3;
                }
                CH_GROUP_SIDE => {
                    // Always map side channels to the multipurpose group.
                    type_ = (plain_idx << 4) | ACT_SIDE;
                    if plain_idx == CH_PLAIN_TOP {
                        ch_map_pos = LEFT_SIDE_CHANNEL_TOP;
                        max_channels = 3;
                    } else {
                        ch_map_pos = LEFT_MULTIPRPS_CHANNEL;
                        max_channels = 2;
                    }
                }
                CH_GROUP_REAR => {
                    type_ = (plain_idx << 4) | ACT_BACK;
                    if plain_idx == CH_PLAIN_TOP {
                        ch_map_pos = LEFT_REAR_CHANNEL_TOP;
                        max_channels = 3;
                    } else {
                        ch_map_pos = LEFT_REAR_CHANNEL;
                        max_channels = 2;
                    }
                }
                CH_GROUP_LFE => {
                    if plain_idx == CH_PLAIN_NORMAL {
                        type_ = ACT_LFE;
                        ch_map_pos = LOW_FREQUENCY_CHANNEL;
                        max_channels = 1;
                    }
                }
                _ => {}
            }

            // Map all channels in this group
            while ch_idx < num_ch_in_grp[grp_idx] {
                let mapped_idx =
                    fdk_ch_map_descr_get_map_value(map_descr, ch as u8, ch_cfg as u32) as usize;
                if ch_idx == max_channels || offset_table.get(ch_map_pos).is_some_and(|&v| v < 255)
                {
                    // No space left in this channel group!
                    if offset_table[LEFT_MULTIPRPS_CHANNEL] == 255 {
                        // Use the multipurpose group:
                        ch_map_pos = LEFT_MULTIPRPS_CHANNEL;
                    } else {
                        fdk_assert!(false, "FDK_ASSERT(0)");
                    }
                }
                // Slots of 8 or more (only with weird channel modes) are outside offsetTable.
                set_at(offset_table, ch_map_pos, mapped_idx as u8);
                set_at(channel_type, mapped_idx, type_);
                set_at(channel_indices, mapped_idx, ch_idx as u8);
                ch_map_pos += 1;
                ch += 1;
                ch_idx += 1;
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Downmix matrix
// ---------------------------------------------------------------------------------------------

/// `dmxInitChannel(mixFactors, mixScales, outCh)` (file-static): initializes one row of the
/// downmix matrix (identity).
pub fn dmx_init_channel(
    mix_factors: &mut [[FixpDmx; 8]; 8],
    mix_scales: &mut [[i32; 8]; 8],
    out_ch: usize,
) {
    for in_ch in 0..8 {
        if in_ch == out_ch {
            mix_factors[out_ch][in_ch] = fl2fxconst_dmx(0.5);
            mix_scales[out_ch][in_ch] = 1;
        } else {
            mix_factors[out_ch][in_ch] = fl2fxconst_dmx(0.0);
            mix_scales[out_ch][in_ch] = 0;
        }
    }
}

/// `dmxClearChannel(mixFactors, mixScales, outCh)` (file-static): clears one row of the downmix
/// matrix.
pub fn dmx_clear_channel(
    mix_factors: &mut [[FixpDmx; 8]; 8],
    mix_scales: &mut [[i32; 8]; 8],
    out_ch: usize,
) {
    mix_factors[out_ch] = [0; 8];
    mix_scales[out_ch] = [0; 8];
}

/// `dmxSetChannel(mixFactors, mixScales, dstCh, srcCh, factor, scale)` (file-static): applies a
/// source row scaled by a mix factor to a destination row. Existing mix factors of the
/// destination row are overwritten where the source row is non-zero.
pub fn dmx_set_channel(
    mix_factors: &mut [[FixpDmx; 8]; 8],
    mix_scales: &mut [[i32; 8]; 8],
    dst_ch: usize,
    src_ch: usize,
    factor: FixpDmx,
    scale: i32,
) {
    for ch in 0..8 {
        if mix_factors[src_ch][ch] != 0 {
            mix_factors[dst_ch][ch] = fx_dbl2fx_sgl(f_mult(mix_factors[src_ch][ch], factor));
            mix_scales[dst_ch][ch] = mix_scales[src_ch][ch].wrapping_add(scale);
        }
    }
}

/// `dmxAddChannel(mixFactors, mixScales, dstCh, srcCh, factor, scale)` (file-static): adds a
/// source row scaled by a mix factor to a destination row. The `FIXP_SGL` shifts and the sum are
/// computed in `int` and truncated, as in C++.
pub fn dmx_add_channel(
    mix_factors: &mut [[FixpDmx; 8]; 8],
    mix_scales: &mut [[i32; 8]; 8],
    dst_ch: usize,
    src_ch: usize,
    factor: FixpDmx,
    scale: i32,
) {
    for ch in 0..8 {
        let mut add_fact: FixpDbl = f_mult(mix_factors[src_ch][ch], factor);
        if add_fact != 0 {
            let mut new_scale = mix_scales[src_ch][ch].wrapping_add(scale);
            if mix_factors[dst_ch][ch] != 0 {
                if new_scale > mix_scales[dst_ch][ch] {
                    let sh = new_scale.wrapping_sub(mix_scales[dst_ch][ch]);
                    mix_factors[dst_ch][ch] =
                        (mix_factors[dst_ch][ch] as i32).wrapping_shr(sh as u32) as i16;
                } else {
                    add_fact = add_fact
                        .wrapping_shr(mix_scales[dst_ch][ch].wrapping_sub(new_scale) as u32);
                    new_scale = mix_scales[dst_ch][ch];
                }
            }
            mix_factors[dst_ch][ch] = (mix_factors[dst_ch][ch] as i32)
                .wrapping_add(fx_dbl2fx_sgl(add_fact) as i32)
                as i16;
            mix_scales[dst_ch][ch] = new_scale;
        }
    }
}

const DMX_METHOD_MPEG_AMD4: i32 = 1;
const DMX_METHOD_MPEG_LEGACY: i32 = 2;
const DMX_METHOD_ARIB_JAPAN: i32 = 4;
const DMX_METHOD_ITU_RECOM: i32 = 8;
const DMX_METHOD_CUSTOM: i32 = 16;

const MAX_SEARCH_START_VAL: i32 = -7;

/// `10^(dmx_gain/80)` of a 7-bit DMX gain index as computed by `getMixFactors()`, with the
/// negative exponent applied to the factor: `(dmxGain, dmxScale)`.
fn dmx_gain(idx: u8) -> (FixpDmx, i32) {
    let sign: i32 = if idx & 0x40 != 0 { -1 } else { 1 };
    let val = (idx & 0x3F) as i32;
    let mut dmx_scale = 0;
    // (FIXP_DBL)(sign * val * (LONG)FL2FXCONST_DBL(0.0125f)): |val| <= 63, no overflow.
    let exp_m = sign * val * fl2fxconst_dbl(0.0125f32 as f64);
    let mut dmx_gain = fx_dbl2fx_sgl(f_ld_pow(
        fl2fxconst_dbl(0.830482023721841f32 as f64),
        2, /* log2(10) */
        exp_m,
        0,
        &mut dmx_scale,
    ));
    // Currently only positive scale factors supported!
    if dmx_scale < 0 {
        dmx_gain = (dmx_gain as i32).wrapping_shr(dmx_scale.wrapping_neg() as u32) as i16;
        dmx_scale = 0;
    }
    (dmx_gain, dmx_scale)
}

/// `getMixFactors(inModeIsCfg, inChMode, outChMode, pParams, pMetaData, mixFactors, pOutScale)`
/// (file-static): creates the downmix factor matrix for the input and output configuration, the
/// user parameters and the metadata. The matrix is normalized to one scale factor, which is
/// written to `p_out_scale`.
pub fn get_mix_factors(
    in_mode_is_cfg: u8,
    mut in_ch_mode: PcmDmxChannelMode,
    out_ch_mode: PcmDmxChannelMode,
    p_params: &PcmDmxUserParams,
    p_meta_data: &DmxBsMetaData,
    mix_factors: &mut [[FixpDmx; 8]; 8],
    p_out_scale: &mut i32,
) -> PcmdmxError {
    let mut err = PCMDMX_OK;
    let mut mix_scales = [[0i32; 8]; 8];
    let mut max_scale: i32 = 0;
    let mut in_ch_cfg: u32 = 0;
    let mut valid = [0u32; 8];

    // Check on a supported output configuration. Add new one only after extensive testing!
    fdk_assert!(
        out_ch_mode == CH_MODE_1_0_0_0
            || out_ch_mode == CH_MODE_2_0_0_0
            || out_ch_mode == CH_MODE_3_0_2_1
            || out_ch_mode == CH_MODE_3_0_4_1
            || out_ch_mode == CH_MODE_5_0_2_1,
        "FDK_ASSERT(0)"
    );

    if in_mode_is_cfg != 0 {
        // Convert channel config to channel mode:
        in_ch_cfg = in_ch_mode as u32;
        match in_ch_cfg {
            1..=6 => in_ch_mode = OUT_CH_MODE_TABLE[in_ch_cfg as usize],
            11 => in_ch_mode = CH_MODE_3_0_3_1,
            12 => in_ch_mode = CH_MODE_3_0_4_1,
            7 | 14 => in_ch_mode = CH_MODE_5_0_2_1,
            _ => fdk_assert!(false, "FDK_ASSERT(0)"),
        }
    }

    // Extract the total number of input channels
    let num_in_channel = (in_ch_mode & 0xF)
        + ((in_ch_mode >> 4) & 0xF)
        + ((in_ch_mode >> 8) & 0xF)
        + ((in_ch_mode >> 12) & 0xF);
    // Extract the total number of output channels
    let num_out_channel = (out_ch_mode & 0xF)
        + ((out_ch_mode >> 4) & 0xF)
        + ((out_ch_mode >> 8) & 0xF)
        + ((out_ch_mode >> 12) & 0xF);

    // MPEG ammendment 4 aka ETSI metadata and fallback mode:

    // Create identity DMX matrix:
    for out_ch in 0..8 {
        dmx_init_channel(mix_factors, &mut mix_scales, out_ch);
    }
    if ((in_ch_mode >> 12) & 0xF) == 0 {
        // Clear empty or wrongly mapped input channel
        dmx_clear_channel(mix_factors, &mut mix_scales, LOW_FREQUENCY_CHANNEL);
    }

    // FIRST STAGE:
    if num_in_channel > SIX_CHANNEL {
        // Always use MPEG equations either with meta data or with default values.
        let mut is_valid_cfg = true;

        // Get factors from meta data
        let d_mix_fact_a = AB_MIX_LVL_VALUE_TAB[p_meta_data.dmix_idx_a as usize];
        let d_mix_scale_a = if p_meta_data.dmix_idx_a == 0 { 1 } else { 0 };
        let d_mix_fact_b = AB_MIX_LVL_VALUE_TAB[p_meta_data.dmix_idx_b as usize];
        let d_mix_scale_b = if p_meta_data.dmix_idx_b == 0 { 1 } else { 0 };

        let (f, s) = (&mut *mix_factors, &mut mix_scales);
        // Check if input is in the list of supported configurations
        let mut case = in_ch_mode;
        if case == CH_MODE_3_2_1_1 || case == CH_MODE_3_2_1_0 {
            // chCfg 11 but with side channels
            is_valid_cfg = false;
            err = PCMDMX_INVALID_MODE;
            case = CH_MODE_3_0_3_1; // FDK_FALLTHROUGH
        }
        if case == CH_MODE_5_0_1_0 || case == CH_MODE_5_0_1_1 {
            dmx_clear_channel(f, s, RIGHT_REAR_CHANNEL); // clear empty input channel
            dmx_set_channel(f, s, RIGHT_REAR_CHANNEL, LEFT_REAR_CHANNEL, fl2fxconst_dmx(0.5), 1);
            dmx_set_channel(f, s, LEFT_REAR_CHANNEL, LEFT_REAR_CHANNEL, fl2fxconst_dmx(0.5), 1);
            case = CH_MODE_5_2_1_0; // FDK_FALLTHROUGH
        }
        if case == CH_MODE_5_2_1_0 {
            is_valid_cfg = false;
            err = PCMDMX_INVALID_MODE;
            case = CH_MODE_5_0_2_1; // FDK_FALLTHROUGH
        }
        match case {
            CH_MODE_3_0_3_1 => {
                // chCfg 11
                // 6.1ch:  C' = C;  L' = L;  R' = R;  LFE' = LFE;
                //         Ls' = Ls*dmix_a_idx + Cs*dmix_b_idx;
                //         Rs' = Rs*dmix_a_idx + Cs*dmix_b_idx;
                dmx_clear_channel(f, s, RIGHT_MULTIPRPS_CHANNEL); // clear empty input channel
                dmx_set_channel(
                    f,
                    s,
                    LEFT_REAR_CHANNEL,
                    LEFT_REAR_CHANNEL,
                    d_mix_fact_a,
                    d_mix_scale_a,
                );
                dmx_set_channel(
                    f,
                    s,
                    LEFT_REAR_CHANNEL,
                    LEFT_MULTIPRPS_CHANNEL,
                    d_mix_fact_b,
                    d_mix_scale_b,
                );
                dmx_set_channel(
                    f,
                    s,
                    RIGHT_REAR_CHANNEL,
                    RIGHT_REAR_CHANNEL,
                    d_mix_fact_a,
                    d_mix_scale_a,
                );
                dmx_set_channel(
                    f,
                    s,
                    RIGHT_REAR_CHANNEL,
                    LEFT_MULTIPRPS_CHANNEL,
                    d_mix_fact_b,
                    d_mix_scale_b,
                );
            }
            CH_MODE_3_0_4_1 => {
                // chCfg 12
                // 7.1ch Surround Back:  C' = C;  L' = L;  R' = R;  LFE' = LFE;
                //                       Ls' = Ls*dmix_a_idx + Lsr*dmix_b_idx;
                //                       Rs' = Rs*dmix_a_idx + Rsr*dmix_b_idx;
                dmx_set_channel(
                    f,
                    s,
                    LEFT_REAR_CHANNEL,
                    LEFT_REAR_CHANNEL,
                    d_mix_fact_a,
                    d_mix_scale_a,
                );
                dmx_set_channel(
                    f,
                    s,
                    LEFT_REAR_CHANNEL,
                    LEFT_MULTIPRPS_CHANNEL,
                    d_mix_fact_b,
                    d_mix_scale_b,
                );
                dmx_set_channel(
                    f,
                    s,
                    RIGHT_REAR_CHANNEL,
                    RIGHT_REAR_CHANNEL,
                    d_mix_fact_a,
                    d_mix_scale_a,
                );
                dmx_set_channel(
                    f,
                    s,
                    RIGHT_REAR_CHANNEL,
                    RIGHT_MULTIPRPS_CHANNEL,
                    d_mix_fact_b,
                    d_mix_scale_b,
                );
            }
            CH_MODE_5_0_2_1 => {
                // chCfg 7 || 14
                if in_ch_cfg == 14 {
                    // 7.1ch Front Height:  C' = C;  Ls' = Ls;  Rs' = Rs;  LFE' = LFE;
                    //                      L' = L*dmix_a_idx + Lv*dmix_b_idx;
                    //                      R' = R*dmix_a_idx + Rv*dmix_b_idx;
                    dmx_set_channel(
                        f,
                        s,
                        LEFT_FRONT_CHANNEL,
                        LEFT_FRONT_CHANNEL,
                        d_mix_fact_a,
                        d_mix_scale_a,
                    );
                    dmx_set_channel(
                        f,
                        s,
                        LEFT_FRONT_CHANNEL,
                        LEFT_MULTIPRPS_CHANNEL,
                        d_mix_fact_b,
                        d_mix_scale_b,
                    );
                    dmx_set_channel(
                        f,
                        s,
                        RIGHT_FRONT_CHANNEL,
                        RIGHT_FRONT_CHANNEL,
                        d_mix_fact_a,
                        d_mix_scale_a,
                    );
                    dmx_set_channel(
                        f,
                        s,
                        RIGHT_FRONT_CHANNEL,
                        RIGHT_MULTIPRPS_CHANNEL,
                        d_mix_fact_b,
                        d_mix_scale_b,
                    );
                } else {
                    // 7.1ch Front:  Ls' = Ls;  Rs' = Rs;  LFE' = LFE;
                    //               C' = C + (Lc+Rc)*dmix_a_idx;
                    //               L' = L + Lc*dmix_b_idx;
                    //               R' = R + Rc*dmix_b_idx;
                    dmx_set_channel(
                        f,
                        s,
                        CENTER_FRONT_CHANNEL,
                        LEFT_MULTIPRPS_CHANNEL,
                        d_mix_fact_a,
                        d_mix_scale_a,
                    );
                    dmx_set_channel(
                        f,
                        s,
                        CENTER_FRONT_CHANNEL,
                        RIGHT_MULTIPRPS_CHANNEL,
                        d_mix_fact_a,
                        d_mix_scale_a,
                    );
                    dmx_set_channel(
                        f,
                        s,
                        LEFT_FRONT_CHANNEL,
                        LEFT_MULTIPRPS_CHANNEL,
                        d_mix_fact_b,
                        d_mix_scale_b,
                    );
                    dmx_set_channel(
                        f,
                        s,
                        LEFT_FRONT_CHANNEL,
                        LEFT_FRONT_CHANNEL,
                        fl2fxconst_dmx(0.5),
                        1,
                    );
                    dmx_set_channel(
                        f,
                        s,
                        RIGHT_FRONT_CHANNEL,
                        RIGHT_MULTIPRPS_CHANNEL,
                        d_mix_fact_b,
                        d_mix_scale_b,
                    );
                    dmx_set_channel(
                        f,
                        s,
                        RIGHT_FRONT_CHANNEL,
                        RIGHT_FRONT_CHANNEL,
                        fl2fxconst_dmx(0.5),
                        1,
                    );
                }
            }
            _ => {
                // Nothing to do. Just use the identity matrix.
                is_valid_cfg = false;
                err = PCMDMX_INVALID_MODE;
            }
        }

        // Add additional DMX gain
        if is_valid_cfg && p_meta_data.dmx_gain_idx5 != 0 {
            // Apply DMX gain 5: 10^(dmx_gain_5/80)
            let (g, sc) = dmx_gain(p_meta_data.dmx_gain_idx5);
            dmx_set_channel(f, s, CENTER_FRONT_CHANNEL, CENTER_FRONT_CHANNEL, g, sc);
            dmx_set_channel(f, s, LEFT_FRONT_CHANNEL, LEFT_FRONT_CHANNEL, g, sc);
            dmx_set_channel(f, s, RIGHT_FRONT_CHANNEL, RIGHT_FRONT_CHANNEL, g, sc);
            dmx_set_channel(f, s, LEFT_REAR_CHANNEL, LEFT_REAR_CHANNEL, g, sc);
            dmx_set_channel(f, s, RIGHT_REAR_CHANNEL, RIGHT_REAR_CHANNEL, g, sc);
            dmx_set_channel(f, s, LOW_FREQUENCY_CHANNEL, LOW_FREQUENCY_CHANNEL, g, sc);
        }

        // Mark the output channels
        valid[CENTER_FRONT_CHANNEL] = 1;
        valid[LEFT_FRONT_CHANNEL] = 1;
        valid[RIGHT_FRONT_CHANNEL] = 1;
        valid[LEFT_REAR_CHANNEL] = 1;
        valid[RIGHT_REAR_CHANNEL] = 1;
        valid[LOW_FREQUENCY_CHANNEL] = 1;

        // Update channel mode for the next stage
        in_ch_mode = CH_MODE_3_0_2_1;
    }

    // For the X (> 6) to 6 channel downmix we had no choice. To mix from 6 to 2 (or 1) channel(s)
    // we have several possibilities (MPEG DSE | MPEG PCE | ITU | ARIB | DLB). Use profile and the
    // metadata available flags to determine which equation to use:

    let mut dmx_method = DMX_METHOD_MPEG_AMD4; // default

    if p_params.dmx_profile == DMX_PRFL_FORCE_MATRIX_MIX
        && (p_meta_data.type_flags & TYPE_PCE_DATA) != 0
    {
        dmx_method = DMX_METHOD_MPEG_LEGACY;
    } else if (p_meta_data.type_flags & (TYPE_DSE_CLEV_DATA | TYPE_DSE_SLEV_DATA)) == 0 {
        match p_params.dmx_profile {
            DMX_PRFL_MATRIX_MIX | DMX_PRFL_FORCE_MATRIX_MIX => {
                if (p_meta_data.type_flags & TYPE_PCE_DATA) != 0 {
                    dmx_method = DMX_METHOD_MPEG_LEGACY;
                }
            }
            DMX_PRFL_ARIB_JAPAN => dmx_method = DMX_METHOD_ARIB_JAPAN,
            // DMX_PRFL_STANDARD and default: dmxMethod = DMX_METHOD_MPEG_AMD4;
            _ => {}
        }
    }

    let s = &mut mix_scales;
    let f = mix_factors;

    // SECOND STAGE:
    if num_out_channel <= TWO_CHANNEL {
        // Create DMX matrix according to input configuration
        match in_ch_mode {
            CH_MODE_2_0_0_0 => {
                // chCfg 2
                // Apply the dual channel mode.
                match p_params.dual_channel_mode {
                    CH1_MODE => {
                        // L' = 0.707 * Ch1;
                        // R' = 0.707 * Ch1;
                        dmx_set_channel(
                            f,
                            s,
                            LEFT_FRONT_CHANNEL,
                            LEFT_FRONT_CHANNEL,
                            fl2fxconst_dmx(0.707),
                            0,
                        );
                        dmx_set_channel(
                            f,
                            s,
                            RIGHT_FRONT_CHANNEL,
                            LEFT_FRONT_CHANNEL,
                            fl2fxconst_dmx(0.707),
                            0,
                        );
                    }
                    CH2_MODE => {
                        // L' = 0.707 * Ch2;
                        // R' = 0.707 * Ch2;
                        dmx_set_channel(
                            f,
                            s,
                            LEFT_FRONT_CHANNEL,
                            RIGHT_FRONT_CHANNEL,
                            fl2fxconst_dmx(0.707),
                            0,
                        );
                        dmx_set_channel(
                            f,
                            s,
                            RIGHT_FRONT_CHANNEL,
                            RIGHT_FRONT_CHANNEL,
                            fl2fxconst_dmx(0.707),
                            0,
                        );
                    }
                    MIXED_MODE => {
                        // L' = 0.5*Ch1 + 0.5*Ch2;
                        // R' = 0.5*Ch1 + 0.5*Ch2;
                        dmx_set_channel(
                            f,
                            s,
                            LEFT_FRONT_CHANNEL,
                            LEFT_FRONT_CHANNEL,
                            fl2fxconst_dmx(0.5),
                            0,
                        );
                        dmx_add_channel(
                            f,
                            s,
                            LEFT_FRONT_CHANNEL,
                            RIGHT_FRONT_CHANNEL,
                            fl2fxconst_dmx(0.5),
                            0,
                        );
                        dmx_set_channel(
                            f,
                            s,
                            RIGHT_FRONT_CHANNEL,
                            LEFT_FRONT_CHANNEL,
                            fl2fxconst_dmx(0.5),
                            0,
                        );
                        dmx_add_channel(
                            f,
                            s,
                            RIGHT_FRONT_CHANNEL,
                            RIGHT_FRONT_CHANNEL,
                            fl2fxconst_dmx(0.5),
                            0,
                        );
                    }
                    // STEREO_MODE and default: Nothing to do
                    _ => {}
                }
            }
            CH_MODE_2_0_1_0 => {
                let s_mix_lvl;
                if dmx_method == DMX_METHOD_ARIB_JAPAN {
                    // L' = 0.707*L + 0.5*S;  R' = 0.707*R + 0.5*S;
                    dmx_set_channel(
                        f,
                        s,
                        LEFT_FRONT_CHANNEL,
                        LEFT_FRONT_CHANNEL,
                        fl2fxconst_dmx(0.707),
                        0,
                    );
                    dmx_set_channel(
                        f,
                        s,
                        RIGHT_FRONT_CHANNEL,
                        RIGHT_FRONT_CHANNEL,
                        fl2fxconst_dmx(0.707),
                        0,
                    );
                    s_mix_lvl = fl2fxconst_dmx(0.5);
                } else {
                    // L' = L + 0.707*S;  R' = R + 0.707*S;
                    s_mix_lvl = fl2fxconst_dmx(0.707);
                }
                dmx_add_channel(f, s, LEFT_FRONT_CHANNEL, LEFT_REAR_CHANNEL, s_mix_lvl, 0);
                dmx_add_channel(f, s, RIGHT_FRONT_CHANNEL, LEFT_REAR_CHANNEL, s_mix_lvl, 0);
            }
            CH_MODE_3_0_0_0 => {
                // chCfg 3
                let c_mix_lvl;
                if dmx_method == DMX_METHOD_ARIB_JAPAN {
                    // L' = 0.707*L + 0.5*C;  R' = 0.707*R + 0.5*C;
                    dmx_set_channel(
                        f,
                        s,
                        LEFT_FRONT_CHANNEL,
                        LEFT_FRONT_CHANNEL,
                        fl2fxconst_dmx(0.707),
                        0,
                    );
                    dmx_set_channel(
                        f,
                        s,
                        RIGHT_FRONT_CHANNEL,
                        RIGHT_FRONT_CHANNEL,
                        fl2fxconst_dmx(0.707),
                        0,
                    );
                    c_mix_lvl = fl2fxconst_dmx(0.5);
                } else {
                    // L' = L + 0.707*C;  R' = R + 0.707*C;
                    c_mix_lvl = fl2fxconst_dmx(0.707);
                }
                dmx_add_channel(f, s, LEFT_FRONT_CHANNEL, CENTER_FRONT_CHANNEL, c_mix_lvl, 0);
                dmx_add_channel(f, s, RIGHT_FRONT_CHANNEL, CENTER_FRONT_CHANNEL, c_mix_lvl, 0);
            }
            CH_MODE_3_0_1_0 => {
                // chCfg 4
                let cs_mix_lvl;
                if dmx_method == DMX_METHOD_ARIB_JAPAN {
                    // L' = 0.707*L + 0.5*C + 0.5*S;  R' = 0.707*R + 0.5*C + 0.5*S;
                    dmx_set_channel(
                        f,
                        s,
                        LEFT_FRONT_CHANNEL,
                        LEFT_FRONT_CHANNEL,
                        fl2fxconst_dmx(0.707),
                        0,
                    );
                    dmx_set_channel(
                        f,
                        s,
                        RIGHT_FRONT_CHANNEL,
                        RIGHT_FRONT_CHANNEL,
                        fl2fxconst_dmx(0.707),
                        0,
                    );
                    cs_mix_lvl = fl2fxconst_dmx(0.5);
                } else {
                    // L' = L + 0.707*C + 0.707*S;
                    // R' = R + 0.707*C + 0.707*S;
                    cs_mix_lvl = fl2fxconst_dmx(0.707);
                }
                dmx_add_channel(f, s, LEFT_FRONT_CHANNEL, CENTER_FRONT_CHANNEL, cs_mix_lvl, 0);
                dmx_add_channel(f, s, LEFT_FRONT_CHANNEL, LEFT_REAR_CHANNEL, cs_mix_lvl, 0);
                dmx_add_channel(f, s, RIGHT_FRONT_CHANNEL, CENTER_FRONT_CHANNEL, cs_mix_lvl, 0);
                dmx_add_channel(f, s, RIGHT_FRONT_CHANNEL, LEFT_REAR_CHANNEL, cs_mix_lvl, 0);
            }
            CH_MODE_3_0_2_0 | CH_MODE_3_0_2_1 => {
                // chCfg 5, chCfg 6
                if dmx_method == DMX_METHOD_ARIB_JAPAN || dmx_method == DMX_METHOD_MPEG_LEGACY {
                    let flev;
                    let (slev_ll, slev_lr, slev_rl, slev_rr);
                    let mtrx_mix_dwn_coef =
                        MPEG_MIX_DOWN_IDX2_COEF[p_meta_data.matrix_mixdown_idx as usize];

                    if p_params.pseudo_surr_mode == FORCE_PS_DMX
                        || (p_params.pseudo_surr_mode == AUTO_PS_DMX
                            && p_meta_data.pseudo_surround == 1)
                    {
                        if dmx_method == DMX_METHOD_ARIB_JAPAN {
                            // 3/2 input: L' = 0.707 * [L+0.707*C-k*Ls-k*Rs];
                            //            R' = 0.707 * [R+0.707*C+k*Ls+k*Rs];
                            flev = MPEG_MIX_DOWN_IDX2_COEF[0]; // a = 0.707
                        } else {
                            // 3/2 input: L' = (1.707+2*A)^-1 * [L+0.707*C-A*Ls-A*Rs];
                            //            R' = (1.707+2*A)^-1 * [R+0.707*C+A*Ls+A*Rs];
                            flev = MPEG_MIX_DOWN_IDX2_PRE_FACT[1]
                                [p_meta_data.matrix_mixdown_idx as usize];
                        }
                        slev_rr = fx_dbl2fx_sgl(f_mult(flev, mtrx_mix_dwn_coef));
                        slev_rl = slev_rr;
                        slev_ll = slev_rl.wrapping_neg();
                        slev_lr = slev_ll;
                    } else {
                        if dmx_method == DMX_METHOD_ARIB_JAPAN {
                            // 3/2 input: L' = 0.707 * [L+0.707*C+k*Ls];
                            //            R' = 0.707 * [R+0.707*C+k*Rs];
                            flev = MPEG_MIX_DOWN_IDX2_COEF[0]; // a = 0.707
                        } else {
                            // 3/2 input: L' = (1.707+A)^-1 * [L+0.707*C+A*Ls];
                            //            R' = (1.707+A)^-1 * [R+0.707*C+A*Rs];
                            flev = MPEG_MIX_DOWN_IDX2_PRE_FACT[0]
                                [p_meta_data.matrix_mixdown_idx as usize];
                        }
                        slev_rr = fx_dbl2fx_sgl(f_mult(flev, mtrx_mix_dwn_coef));
                        slev_ll = slev_rr;
                        slev_rl = 0;
                        slev_lr = 0;
                    }
                    // common factor
                    let clev =
                        fx_dbl2fx_sgl(f_mult(flev, MPEG_MIX_DOWN_IDX2_COEF[0] /* 0.707 */));

                    dmx_set_channel(f, s, LEFT_FRONT_CHANNEL, LEFT_FRONT_CHANNEL, flev, 0);
                    dmx_add_channel(f, s, LEFT_FRONT_CHANNEL, CENTER_FRONT_CHANNEL, clev, 0);
                    dmx_add_channel(f, s, LEFT_FRONT_CHANNEL, LEFT_REAR_CHANNEL, slev_ll, 0);
                    dmx_add_channel(f, s, LEFT_FRONT_CHANNEL, RIGHT_REAR_CHANNEL, slev_lr, 0);

                    dmx_set_channel(f, s, RIGHT_FRONT_CHANNEL, RIGHT_FRONT_CHANNEL, flev, 0);
                    dmx_add_channel(f, s, RIGHT_FRONT_CHANNEL, CENTER_FRONT_CHANNEL, clev, 0);
                    dmx_add_channel(f, s, RIGHT_FRONT_CHANNEL, LEFT_REAR_CHANNEL, slev_rl, 0);
                    dmx_add_channel(f, s, RIGHT_FRONT_CHANNEL, RIGHT_REAR_CHANNEL, slev_rr, 0);
                } else {
                    // DMX_METHOD_MPEG_AMD4 and default
                    // Get factors from meta data
                    let c_mix_lvl = AB_MIX_LVL_VALUE_TAB[p_meta_data.c_lev_idx as usize];
                    let c_mix_scale = if p_meta_data.c_lev_idx == 0 { 1 } else { 0 };
                    let s_mix_lvl = AB_MIX_LVL_VALUE_TAB[p_meta_data.s_lev_idx as usize];
                    let s_mix_scale = if p_meta_data.s_lev_idx == 0 { 1 } else { 0 };
                    let l_mix_lvl = LFE_MIX_LVL_VALUE_TAB[p_meta_data.dmix_idx_lfe as usize];
                    let l_mix_scale = if p_meta_data.dmix_idx_lfe <= 1 {
                        2
                    } else if p_meta_data.dmix_idx_lfe <= 5 {
                        1
                    } else {
                        0
                    };
                    // Setup the DMX matrix
                    if p_params.pseudo_surr_mode == FORCE_PS_DMX
                        || (p_params.pseudo_surr_mode == AUTO_PS_DMX
                            && p_meta_data.pseudo_surround == 1)
                    {
                        // L' = L + C*clev - (Ls+Rs)*slev + LFE*lflev;
                        // R' = R + C*clev + (Ls+Rs)*slev + LFE*lflev;
                        dmx_add_channel(
                            f,
                            s,
                            LEFT_FRONT_CHANNEL,
                            CENTER_FRONT_CHANNEL,
                            c_mix_lvl,
                            c_mix_scale,
                        );
                        dmx_add_channel(
                            f,
                            s,
                            LEFT_FRONT_CHANNEL,
                            LEFT_REAR_CHANNEL,
                            s_mix_lvl.wrapping_neg(),
                            s_mix_scale,
                        );
                        dmx_add_channel(
                            f,
                            s,
                            LEFT_FRONT_CHANNEL,
                            RIGHT_REAR_CHANNEL,
                            s_mix_lvl.wrapping_neg(),
                            s_mix_scale,
                        );
                        dmx_add_channel(
                            f,
                            s,
                            LEFT_FRONT_CHANNEL,
                            LOW_FREQUENCY_CHANNEL,
                            l_mix_lvl,
                            l_mix_scale,
                        );
                        dmx_add_channel(
                            f,
                            s,
                            RIGHT_FRONT_CHANNEL,
                            CENTER_FRONT_CHANNEL,
                            c_mix_lvl,
                            c_mix_scale,
                        );
                        dmx_add_channel(
                            f,
                            s,
                            RIGHT_FRONT_CHANNEL,
                            LEFT_REAR_CHANNEL,
                            s_mix_lvl,
                            s_mix_scale,
                        );
                        dmx_add_channel(
                            f,
                            s,
                            RIGHT_FRONT_CHANNEL,
                            RIGHT_REAR_CHANNEL,
                            s_mix_lvl,
                            s_mix_scale,
                        );
                        dmx_add_channel(
                            f,
                            s,
                            RIGHT_FRONT_CHANNEL,
                            LOW_FREQUENCY_CHANNEL,
                            l_mix_lvl,
                            l_mix_scale,
                        );
                    } else {
                        // L' = L + C*clev + Ls*slev + LFE*llev;
                        // R' = R + C*clev + Rs*slev + LFE*llev;
                        dmx_add_channel(
                            f,
                            s,
                            LEFT_FRONT_CHANNEL,
                            CENTER_FRONT_CHANNEL,
                            c_mix_lvl,
                            c_mix_scale,
                        );
                        dmx_add_channel(
                            f,
                            s,
                            LEFT_FRONT_CHANNEL,
                            LEFT_REAR_CHANNEL,
                            s_mix_lvl,
                            s_mix_scale,
                        );
                        dmx_add_channel(
                            f,
                            s,
                            LEFT_FRONT_CHANNEL,
                            LOW_FREQUENCY_CHANNEL,
                            l_mix_lvl,
                            l_mix_scale,
                        );
                        dmx_add_channel(
                            f,
                            s,
                            RIGHT_FRONT_CHANNEL,
                            CENTER_FRONT_CHANNEL,
                            c_mix_lvl,
                            c_mix_scale,
                        );
                        dmx_add_channel(
                            f,
                            s,
                            RIGHT_FRONT_CHANNEL,
                            RIGHT_REAR_CHANNEL,
                            s_mix_lvl,
                            s_mix_scale,
                        );
                        dmx_add_channel(
                            f,
                            s,
                            RIGHT_FRONT_CHANNEL,
                            LOW_FREQUENCY_CHANNEL,
                            l_mix_lvl,
                            l_mix_scale,
                        );
                    }

                    // Add additional DMX gain
                    if p_meta_data.dmx_gain_idx2 != 0 {
                        // Apply DMX gain 2: 10^(dmx_gain_2/80)
                        let (g, sc) = dmx_gain(p_meta_data.dmx_gain_idx2);
                        dmx_set_channel(f, s, LEFT_FRONT_CHANNEL, LEFT_FRONT_CHANNEL, g, sc);
                        dmx_set_channel(f, s, RIGHT_FRONT_CHANNEL, RIGHT_FRONT_CHANNEL, g, sc);
                    }
                }
            }
            _ => {
                // This configuration does not fit to any known downmix equation!
                err = PCMDMX_INVALID_MODE;
            }
        }

        // Mark the output channels
        valid = [0; 8];
        valid[LEFT_FRONT_CHANNEL] = 1;
        valid[RIGHT_FRONT_CHANNEL] = 1;
    }

    if num_out_channel == ONE_CHANNEL {
        let mut mono_mix_level: FixpDmx;
        let mono_mix_scale;

        dmx_clear_channel(f, s, CENTER_FRONT_CHANNEL); // C is not in the mix

        if dmx_method == DMX_METHOD_MPEG_LEGACY {
            // C' = (3+2*A)^-1 * [C+L+R+A*Ls+A+Rs];
            mono_mix_level =
                MPEG_MIX_DOWN_IDX2_PRE_FACT[2][p_meta_data.matrix_mixdown_idx as usize];

            f[CENTER_FRONT_CHANNEL][CENTER_FRONT_CHANNEL] = mono_mix_level;
            f[CENTER_FRONT_CHANNEL][LEFT_FRONT_CHANNEL] = mono_mix_level;
            f[CENTER_FRONT_CHANNEL][RIGHT_FRONT_CHANNEL] = mono_mix_level;
            mono_mix_level = fx_dbl2fx_sgl(f_mult(
                mono_mix_level,
                MPEG_MIX_DOWN_IDX2_COEF[p_meta_data.matrix_mixdown_idx as usize],
            ));
            f[CENTER_FRONT_CHANNEL][LEFT_REAR_CHANNEL] = mono_mix_level;
            f[CENTER_FRONT_CHANNEL][RIGHT_REAR_CHANNEL] = mono_mix_level;
        } else {
            match dmx_method {
                DMX_METHOD_MPEG_AMD4 => {
                    // C' = L + R;
                    mono_mix_level = fl2fxconst_dmx(0.5);
                    mono_mix_scale = 1;
                }
                _ => {
                    // C' = 0.5*L + 0.5*R;
                    mono_mix_level = fl2fxconst_dmx(0.5);
                    mono_mix_scale = 0;
                }
            }
            dmx_set_channel(
                f,
                s,
                CENTER_FRONT_CHANNEL,
                LEFT_FRONT_CHANNEL,
                mono_mix_level,
                mono_mix_scale,
            );
            dmx_add_channel(
                f,
                s,
                CENTER_FRONT_CHANNEL,
                RIGHT_FRONT_CHANNEL,
                mono_mix_level,
                mono_mix_scale,
            );
        }

        // Mark the output channel
        valid = [0; 8];
        valid[CENTER_FRONT_CHANNEL] = 1;
    }

    {
        let mut ch_sum = [0i32; 8];
        let mut ch_sum_max = MAX_SEARCH_START_VAL;

        // Determine the current maximum scale factor
        for out_ch in 0..8 {
            if valid[out_ch] != 0 {
                for in_ch in 0..8 {
                    if s[out_ch][in_ch] > max_scale {
                        // Store the new maximum
                        max_scale = s[out_ch][in_ch];
                    }
                }
            }
        }

        // Individualy analyse output chanal levels
        for out_ch in 0..8 {
            ch_sum[out_ch] = MAX_SEARCH_START_VAL;
            if valid[out_ch] != 0 {
                let mut ovrflw_prot_scale = 0;

                // Accumulate all factors for each output channel
                ch_sum[out_ch] = 0;
                for in_ch in 0..8 {
                    // SHORT addFact; the shifts are computed in int and truncated to SHORT.
                    let mut add_fact: i16 = f[out_ch][in_ch];
                    if s[out_ch][in_ch] <= max_scale {
                        add_fact = (add_fact as i32)
                            .wrapping_shr(max_scale.wrapping_sub(s[out_ch][in_ch]) as u32)
                            as i16;
                    } else {
                        add_fact = (add_fact as i32)
                            .wrapping_shl(s[out_ch][in_ch].wrapping_sub(max_scale) as u32)
                            as i16;
                    }
                    ch_sum[out_ch] += add_fact as i32;
                }
                if ch_sum[out_ch] > MAXVAL_SGL {
                    while ch_sum[out_ch] > MAXVAL_SGL {
                        ovrflw_prot_scale += 1;
                        ch_sum[out_ch] >>= 1;
                    }
                } else if ch_sum[out_ch] > 0 {
                    while (ch_sum[out_ch] << 1) <= MAXVAL_SGL {
                        ovrflw_prot_scale -= 1;
                        ch_sum[out_ch] <<= 1;
                    }
                }
                // Store the differential scaling in the same array
                ch_sum[out_ch] = ovrflw_prot_scale;
            }
        }

        for out_ch in 0..8 {
            if valid[out_ch] != 0 && ch_sum[out_ch] > ch_sum_max {
                // Store the new maximum
                ch_sum_max = ch_sum[out_ch];
            }
        }
        max_scale = f_max(max_scale.wrapping_add(ch_sum_max), 0);

        // Normalize all factors
        for out_ch in 0..8 {
            if valid[out_ch] != 0 {
                for in_ch in 0..8 {
                    if f[out_ch][in_ch] != 0 {
                        if s[out_ch][in_ch] <= max_scale {
                            f[out_ch][in_ch] = (f[out_ch][in_ch] as i32)
                                .wrapping_shr(max_scale.wrapping_sub(s[out_ch][in_ch]) as u32)
                                as i16;
                        } else {
                            f[out_ch][in_ch] = (f[out_ch][in_ch] as i32)
                                .wrapping_shl(s[out_ch][in_ch].wrapping_sub(max_scale) as u32)
                                as i16;
                        }
                        s[out_ch][in_ch] = max_scale;
                    }
                }
            }
        }
    }

    // return the scale factor
    *p_out_scale = max_scale;

    err
}

// ---------------------------------------------------------------------------------------------
// Module interface
// ---------------------------------------------------------------------------------------------

/// `pcmDmx_Open(pSelf)`: opens and initializes an instance of the PCM downmix module.
pub fn pcm_dmx_open(p_self: Option<&mut Option<HandlePcmDownmix>>) -> PcmdmxError {
    let Some(p_self) = p_self else {
        return PCMDMX_INVALID_HANDLE;
    };

    *p_self = None;

    // GetPcmDmxInstance(0): FDKcalloc
    let mut self_ = Box::<PcmDmxInstance>::default();

    // Reset the full instance
    pcm_dmx_reset(Some(&mut self_), PCMDMX_RESET_FULL);

    *p_self = Some(self_);

    PCMDMX_OK
}

/// `pcmDmx_Reset(self, flags)`: resets the user parameters (`PCMDMX_RESET_PARAMS`) and/or the
/// metadata (`PCMDMX_RESET_BS_DATA`). The downmix profile is not reset.
pub fn pcm_dmx_reset(self_: Option<&mut PcmDmxInstance>, flags: u32) -> PcmdmxError {
    let Some(self_) = self_ else {
        return PCMDMX_INVALID_HANDLE;
    };

    if flags & PCMDMX_RESET_PARAMS != 0 {
        let p_params = &mut self_.user_params;

        p_params.dual_channel_mode = STEREO_MODE;
        p_params.pseudo_surr_mode = NEVER_DO_PS_DMX;
        p_params.num_out_channels_max = 6;
        p_params.num_out_channels_min = 0;
        p_params.frame_delay = 0;
        p_params.expiry_frame = 0;

        self_.apply_processing = 0;
    }

    if flags & PCMDMX_RESET_BS_DATA != 0 {
        // Init all slots with a default set
        for slot in self_.bs_meta_data.iter_mut() {
            *slot = DFLT_META_DATA;
        }
    }

    PCMDMX_OK
}

/// `pcmDmx_SetParam(self, param, value)`: sets one parameter. The value is checked before the
/// handle, as in C++.
pub fn pcm_dmx_set_param(
    self_: Option<&mut PcmDmxInstance>,
    param: PcmdmxParam,
    value: i32,
) -> PcmdmxError {
    match param {
        DMX_PROFILE_SETTING => {
            match value {
                DMX_PRFL_STANDARD
                | DMX_PRFL_MATRIX_MIX
                | DMX_PRFL_FORCE_MATRIX_MIX
                | DMX_PRFL_ARIB_JAPAN => {}
                _ => return PCMDMX_UNABLE_TO_SET_PARAM,
            }
            let Some(self_) = self_ else {
                return PCMDMX_INVALID_HANDLE;
            };
            self_.user_params.dmx_profile = value;
        }

        DMX_BS_DATA_EXPIRY_FRAME => {
            let Some(self_) = self_ else {
                return PCMDMX_INVALID_HANDLE;
            };
            self_.user_params.expiry_frame = if value > 0 { value as u32 } else { 0 };
        }

        DMX_BS_DATA_DELAY => {
            if value > 1 || value < 0 {
                return PCMDMX_UNABLE_TO_SET_PARAM;
            }
            let Some(self_) = self_ else {
                return PCMDMX_INVALID_HANDLE;
            };
            self_.user_params.frame_delay = value as u8;
        }

        MIN_NUMBER_OF_OUTPUT_CHANNELS => {
            match value {
                // supported output channels
                -1 | 0 | ONE_CHANNEL | TWO_CHANNEL | SIX_CHANNEL | EIGHT_CHANNEL => {}
                _ => return PCMDMX_UNABLE_TO_SET_PARAM,
            }
            let Some(self_) = self_ else {
                return PCMDMX_INVALID_HANDLE;
            };
            // Store the new value
            self_.user_params.num_out_channels_min = if value > 0 { value as i16 } else { -1 };
            if value > 0
                && self_.user_params.num_out_channels_max > 0
                && value > self_.user_params.num_out_channels_max as i32
            {
                // MIN > MAX would be an invalid state. Thus set MAX = MIN in this case.
                self_.user_params.num_out_channels_max = self_.user_params.num_out_channels_min;
            }
        }

        MAX_NUMBER_OF_OUTPUT_CHANNELS => {
            match value {
                // supported output channels
                -1 | 0 | ONE_CHANNEL | TWO_CHANNEL | SIX_CHANNEL | EIGHT_CHANNEL => {}
                _ => return PCMDMX_UNABLE_TO_SET_PARAM,
            }
            let Some(self_) = self_ else {
                return PCMDMX_INVALID_HANDLE;
            };
            // Store the new value
            self_.user_params.num_out_channels_max = if value > 0 { value as i16 } else { -1 };
            if value > 0 && value < self_.user_params.num_out_channels_min as i32 {
                // MAX < MIN would be an invalid state. Thus set MIN = MAX in this case.
                self_.user_params.num_out_channels_min = self_.user_params.num_out_channels_max;
            }
        }

        DMX_DUAL_CHANNEL_MODE => {
            match value {
                STEREO_MODE | CH1_MODE | CH2_MODE | MIXED_MODE => {}
                _ => return PCMDMX_UNABLE_TO_SET_PARAM,
            }
            let Some(self_) = self_ else {
                return PCMDMX_INVALID_HANDLE;
            };
            self_.user_params.dual_channel_mode = value;
            // Force processing if necessary.
            self_.apply_processing = if value != STEREO_MODE { 1 } else { 0 };
        }

        DMX_PSEUDO_SURROUND_MODE => {
            match value {
                NEVER_DO_PS_DMX | AUTO_PS_DMX | FORCE_PS_DMX => {}
                _ => return PCMDMX_UNABLE_TO_SET_PARAM,
            }
            let Some(self_) = self_ else {
                return PCMDMX_INVALID_HANDLE;
            };
            self_.user_params.pseudo_surr_mode = value;
        }

        _ => return PCMDMX_UNKNOWN_PARAM,
    }

    PCMDMX_OK
}

/// `pcmDmx_GetParam(self, param, pValue)`: gets one parameter value.
pub fn pcm_dmx_get_param(
    self_: Option<&PcmDmxInstance>,
    param: PcmdmxParam,
    p_value: Option<&mut i32>,
) -> PcmdmxError {
    let (Some(self_), Some(p_value)) = (self_, p_value) else {
        return PCMDMX_INVALID_HANDLE;
    };
    let p_usr_params = &self_.user_params;

    *p_value = match param {
        DMX_PROFILE_SETTING => p_usr_params.dmx_profile,
        DMX_BS_DATA_EXPIRY_FRAME => p_usr_params.expiry_frame as i32,
        DMX_BS_DATA_DELAY => p_usr_params.frame_delay as i32,
        MIN_NUMBER_OF_OUTPUT_CHANNELS => p_usr_params.num_out_channels_min as i32,
        MAX_NUMBER_OF_OUTPUT_CHANNELS => p_usr_params.num_out_channels_max as i32,
        DMX_DUAL_CHANNEL_MODE => p_usr_params.dual_channel_mode,
        DMX_PSEUDO_SURROUND_MODE => p_usr_params.pseudo_surr_mode,
        _ => return PCMDMX_UNKNOWN_PARAM,
    };

    PCMDMX_OK
}

/// `MAX_DSE_ANC_BYTES` (15 bytes)
pub const MAX_DSE_ANC_BYTES: u32 = 16;
/// `ANC_DATA_SYNC_BYTE`: ancillary data sync byte.
pub const ANC_DATA_SYNC_BYTE: u32 = 0xBC;

/// `pcmDmx_Parse(self, hBs, ancDataBits, isMpeg2)`: reads the DMX metadata (ETSI TS 101 154 or
/// ISO/IEC 14496-3:2009/Amd.4) from a data stream element. The fields are stored into the
/// metadata as they are read; only the new-data flags and the expiry counter depend on the
/// final check of the amount of data read.
pub fn pcm_dmx_parse(
    self_: Option<&mut PcmDmxInstance>,
    h_bs: Option<&mut FdkBitStream>,
    anc_data_bits: u32,
    is_mpeg2: i32,
) -> PcmdmxError {
    let mut error_status = PCMDMX_OK;

    let mut skip4_dmx: u32 = 0;
    let mut skip4_ext: u32 = 0;
    let mut ext_data_avail = 0u32;
    let mut found_new_data: u32 = 0;
    let min_anc_bits: u32 = (if is_mpeg2 != 0 { 5 } else { 3 }) * 8;

    let (Some(self_), Some(h_bs)) = (self_, h_bs) else {
        return PCMDMX_INVALID_HANDLE;
    };

    // sanity checks
    if anc_data_bits < min_anc_bits || anc_data_bits > fdk_get_valid_bits(h_bs) {
        return PCMDMX_CORRUPT_ANC_DATA;
    }

    let p_bs_meta_data = &mut self_.bs_meta_data[0];

    if is_mpeg2 != 0 {
        // skip DVD ancillary data
        fdk_push_for(h_bs, 16);
    }

    // check sync word
    if fdk_read_bits(h_bs, 8) != ANC_DATA_SYNC_BYTE {
        return PCMDMX_CORRUPT_ANC_DATA;
    }

    // skip MPEG audio type and Dolby surround mode
    fdk_push_for(h_bs, 4);

    if is_mpeg2 != 0 {
        // int numAncBytes =
        fdk_read_bits(h_bs, 4);
        // advanced dynamic range control
        if fdk_read_bit(h_bs) != 0 {
            skip4_dmx += 24;
        }
        // dialog normalization
        if fdk_read_bit(h_bs) != 0 {
            skip4_dmx += 8;
        }
        // reproduction_level
        if fdk_read_bit(h_bs) != 0 {
            skip4_dmx += 8;
        }
    } else {
        fdk_push_for(h_bs, 2); // drc presentation mode
        p_bs_meta_data.pseudo_surround = fdk_read_bit(h_bs) as i8;
        fdk_push_for(h_bs, 4); // reserved bits
    }

    // downmixing levels MPEGx status
    let dmx_lvl_avail = fdk_read_bit(h_bs);

    if is_mpeg2 != 0 {
        // scale factor CRC status
        if fdk_read_bit(h_bs) != 0 {
            skip4_ext += 16;
        }
    } else {
        // ancillary data extension status
        ext_data_avail = fdk_read_bit(h_bs);
    }

    // audio coding and compression status
    if fdk_read_bit(h_bs) != 0 {
        skip4_ext += 16;
    }
    // coarse grain timecode status
    if fdk_read_bit(h_bs) != 0 {
        skip4_ext += 16;
    }
    // fine grain timecode status
    if fdk_read_bit(h_bs) != 0 {
        skip4_ext += 16;
    }

    // skip the useless data to get to the DMX levels
    fdk_push_for(h_bs, skip4_dmx);

    // downmix_levels_MPEGX
    if dmx_lvl_avail != 0 {
        if fdk_read_bit(h_bs) != 0 {
            // center_mix_level_on
            p_bs_meta_data.c_lev_idx = fdk_read_bits(h_bs, 3) as u8;
            found_new_data |= TYPE_DSE_CLEV_DATA;
        } else {
            fdk_read_bits(h_bs, 3);
        }
        if fdk_read_bit(h_bs) != 0 {
            // surround_mix_level_on
            p_bs_meta_data.s_lev_idx = fdk_read_bits(h_bs, 3) as u8;
            found_new_data |= TYPE_DSE_SLEV_DATA;
        } else {
            fdk_read_bits(h_bs, 3);
        }
    }

    // skip the useless data to get to the ancillary data extension
    fdk_push_for(h_bs, skip4_ext);

    // anc data extension (MPEG-4 only)
    if ext_data_avail != 0 {
        fdk_read_bit(h_bs); // reserved bit
        let ext_dmx_lvl_st = fdk_read_bit(h_bs);
        let ext_dmx_gain_st = fdk_read_bit(h_bs);
        let ext_dmx_lfe_st = fdk_read_bit(h_bs);
        fdk_read_bits(h_bs, 4); // reserved bits

        if ext_dmx_lvl_st != 0 {
            p_bs_meta_data.dmix_idx_a = fdk_read_bits(h_bs, 3) as u8;
            p_bs_meta_data.dmix_idx_b = fdk_read_bits(h_bs, 3) as u8;
            fdk_read_bits(h_bs, 2); // reserved bits
            found_new_data |= TYPE_DSE_DMIX_AB_DATA;
        }
        if ext_dmx_gain_st != 0 {
            p_bs_meta_data.dmx_gain_idx5 = fdk_read_bits(h_bs, 7) as u8;
            fdk_read_bit(h_bs); // reserved bit
            p_bs_meta_data.dmx_gain_idx2 = fdk_read_bits(h_bs, 7) as u8;
            fdk_read_bit(h_bs); // reserved bit
            found_new_data |= TYPE_DSE_DMX_GAIN_DATA;
        }
        if ext_dmx_lfe_st != 0 {
            p_bs_meta_data.dmix_idx_lfe = fdk_read_bits(h_bs, 4) as u8;
            fdk_read_bits(h_bs, 4); // reserved bits
            found_new_data |= TYPE_DSE_DMIX_LFE_DATA;
        }
    }

    // final sanity check on the amount of read data
    if (fdk_get_valid_bits(h_bs) as i32) < 0 {
        error_status = PCMDMX_CORRUPT_ANC_DATA;
    }

    if error_status == PCMDMX_OK && found_new_data != 0 {
        // announce new data
        p_bs_meta_data.type_flags |= found_new_data;
        // reset expiry counter
        p_bs_meta_data.expiry_count = 0;
    }

    error_status
}

/// `pcmDmx_ReadDvbAncData(self, pAncDataBuf, ancDataBytes, isMpeg2)`: reads the DMX metadata
/// from an ancillary data buffer.
///
/// The reference reads through a bitstream over `MAX_DSE_ANC_BYTES` (16) bytes of
/// `pAncDataBuf`, whatever `ancDataBytes` is (the reads wrap around within these 16 bytes). The
/// port copies the first 16 bytes of the slice; bytes beyond a shorter slice read as 0
/// (PORTING.md, "Deviations from the reference").
pub fn pcm_dmx_read_dvb_anc_data(
    self_: Option<&mut PcmDmxInstance>,
    p_anc_data_buf: Option<&[u8]>,
    anc_data_bytes: u32,
    is_mpeg2: i32,
) -> PcmdmxError {
    let Some(self_) = self_ else {
        return PCMDMX_INVALID_HANDLE;
    };

    // sanity checks
    let Some(p_anc_data_buf) = p_anc_data_buf else {
        return PCMDMX_CORRUPT_ANC_DATA;
    };
    if anc_data_bytes == 0 {
        return PCMDMX_CORRUPT_ANC_DATA;
    }

    let buf = BitMem::new(MAX_DSE_ANC_BYTES as usize);
    buf.write(0, &p_anc_data_buf[..p_anc_data_buf.len().min(MAX_DSE_ANC_BYTES as usize)]);

    let mut bs = FdkBitStream::default();
    fdk_init_bit_stream(
        &mut bs,
        &buf,
        MAX_DSE_ANC_BYTES,
        anc_data_bytes.wrapping_mul(8),
        BS_READER,
    );

    pcm_dmx_parse(Some(self_), Some(&mut bs), anc_data_bytes.wrapping_mul(8), is_mpeg2)
}

/// `pcmDmx_SetMatrixMixdownFromPce(self, matrixMixdownPresent, matrixMixdownIdx,
/// pseudoSurroundEnable)`: sets the matrix mixdown information extracted from the PCE.
pub fn pcm_dmx_set_matrix_mixdown_from_pce(
    self_: Option<&mut PcmDmxInstance>,
    matrix_mixdown_present: i32,
    matrix_mixdown_idx: i32,
    pseudo_surround_enable: i32,
) -> PcmdmxError {
    let Some(self_) = self_ else {
        return PCMDMX_INVALID_HANDLE;
    };

    let p_bs_meta_data = &mut self_.bs_meta_data[0];

    if matrix_mixdown_present != 0 {
        p_bs_meta_data.pseudo_surround = if pseudo_surround_enable != 0 { 1 } else { 0 };
        p_bs_meta_data.matrix_mixdown_idx = (matrix_mixdown_idx & 0x03) as u8;
        p_bs_meta_data.type_flags |= TYPE_PCE_DATA;
        // Reset expiry counter
        p_bs_meta_data.expiry_count = 0;
    }

    PCMDMX_OK
}

/// Reads `buf[i]`; 0 outside the buffer.
#[inline]
fn rd(buf: &[DmxPcm], i: isize) -> DmxPcm {
    if i >= 0 {
        buf.get(i as usize).copied().unwrap_or(0)
    } else {
        0
    }
}

/// Writes `buf[i]`; dropped outside the buffer.
#[inline]
fn wr(buf: &mut [DmxPcm], i: isize, v: DmxPcm) {
    if i >= 0 {
        if let Some(e) = buf.get_mut(i as usize) {
            *e = v;
        }
    }
}

/// `pcmDmx_ApplyFrame(self, pPcmBuf, pcmBufSize, frameSize, nChannels, fInterleaved,
/// channelType, channelIndices, mapDescr, pDmxOutScale)`: creates a mixdown, bypasses or extends
/// the output signal depending on the module's settings and the given input configuration.
///
/// * `p_pcm_buf`: the time signal, processed in place (`pcm_buf_size` is the size the C++ caller
///   states; the accesses are bounded by the slice, see the module documentation)
/// * `n_channels`: the number of input channels; receives the number of output channels
/// * `channel_type`, `channel_indices`: the channel description of the input; receives the one
///   of the output
/// * `p_dmx_out_scale`: receives the scale factor that has to be applied on all samples
///   afterwards; with `None` (`NULL`), the scaling is done internally
///
/// For a negative `*nChannels` the port returns `PCMDMX_INVALID_CH_CONFIG` (PORTING.md,
/// "Deviations from the reference").
pub fn pcm_dmx_apply_frame(
    self_: Option<&mut PcmDmxInstance>,
    p_pcm_buf: Option<&mut [DmxPcm]>,
    pcm_buf_size: i32,
    frame_size: u32,
    n_channels: Option<&mut i32>,
    f_interleaved: i32,
    channel_type: Option<&mut [AudioChannelType; 8]>,
    channel_indices: Option<&mut [u8; 8]>,
    map_descr: Option<&FdkChannelMapDescr>,
    p_dmx_out_scale: Option<&mut i32>,
) -> PcmdmxError {
    let mut error_status = PCMDMX_OK;
    let mut dev_null: i32 = 0; // Just a dummy to avoid a lot of branches in the code
    let dmx_max_scale: i32;
    let mut dmx_scale: i32;
    let mut in_offset_table = [0u8; 8];

    let (Some(self_), Some(n_channels), Some(channel_type), Some(channel_indices)) =
        (self_, n_channels, channel_type, channel_indices)
    else {
        return PCMDMX_INVALID_HANDLE;
    };
    if fdk_ch_map_descr_is_valid(map_descr) == 0 {
        return PCMDMX_INVALID_HANDLE;
    }
    let Some(map_descr) = map_descr else {
        return PCMDMX_INVALID_HANDLE;
    };

    // Init the output scaling
    dmx_scale = 0;
    let p_dmx_out_scale: &mut i32 = match p_dmx_out_scale {
        Some(p) => {
            // Avoid final scaling internally and hand it to the outside world.
            *p = 0;
            dmx_max_scale = 3;
            p
        }
        None => {
            // Apply the scaling internally.
            dmx_max_scale = 0;
            &mut dev_null // redirect to temporal stack memory
        }
    };

    let num_in_channels = *n_channels;

    // Perform some input sanity checks
    let Some(pcm) = p_pcm_buf else {
        return PCMDMX_INVALID_ARGUMENT;
    };
    if frame_size == 0 {
        return PCMDMX_INVALID_ARGUMENT;
    }
    if num_in_channels == 0 {
        return PCMDMX_INVALID_ARGUMENT;
    }
    if num_in_channels > 8 {
        return PCMDMX_INVALID_CH_CONFIG;
    }

    let p_param = self_.user_params;

    // Check on misconfiguration
    fdk_assert!(
        p_param.num_out_channels_max <= 0
            || p_param.num_out_channels_max >= p_param.num_out_channels_min,
        "FDK_ASSERT(numOutChannelsMax <= 0 || numOutChannelsMax >= numOutChannelsMin)"
    );

    // Determine if the module has to do processing
    if self_.apply_processing == 0
        && (p_param.num_out_channels_max <= 0
            || p_param.num_out_channels_max as i32 >= num_in_channels)
        && (p_param.num_out_channels_min as i32) <= num_in_channels
    {
        // Nothing to do
        return error_status;
    }

    // Determine the number of output channels
    let num_out_channels: i32 = if p_param.num_out_channels_max > 0
        && num_in_channels > p_param.num_out_channels_max as i32
    {
        p_param.num_out_channels_max as i32
    } else if num_in_channels < p_param.num_out_channels_min as i32 {
        p_param.num_out_channels_min as i32
    } else {
        num_in_channels
    };

    // Check I/O buffer size
    if (pcm_buf_size as u32) < (num_out_channels as u32).wrapping_mul(frame_size) {
        return PCMDMX_OUTPUT_BUFFER_TOO_SMALL;
    }

    if num_in_channels < 0 {
        // Deviation: the reference would evaluate (UINT)numInChannels channels.
        return PCMDMX_INVALID_CH_CONFIG;
    }

    let dual_channel_mode = p_param.dual_channel_mode;

    // Analyse input channel configuration and get channel offset table that can be accessed
    // with the fixed channel labels.
    let mut in_ch_mode: PcmDmxChannelMode = CH_MODE_UNDEFINED;
    error_status = get_channel_mode(
        num_in_channels as u32,
        channel_type,
        channel_indices,
        &mut in_offset_table,
        &mut in_ch_mode,
    );
    if pcmdmx_is_fatal_error(error_status) != 0 || in_ch_mode == CH_MODE_UNDEFINED {
        // We don't need to restore because the channel configuration has not been changed. Just
        // exit.
        return PCMDMX_INVALID_CH_CONFIG;
    }

    // Set input stride and offset
    let mut in_stride: isize;
    let mut offset: isize;
    if f_interleaved != 0 {
        in_stride = num_in_channels as isize;
        offset = 1; // Channel specific offset factor
    } else {
        in_stride = 1;
        offset = frame_size as i32 as isize; // Channel specific offset factor
    }
    let frame_size_m1 = frame_size.wrapping_sub(1) as i32 as isize;

    // Reset downmix meta data if necessary
    if p_param.expiry_frame > 0 && {
        self_.bs_meta_data[0].expiry_count = self_.bs_meta_data[0].expiry_count.wrapping_add(1);
        self_.bs_meta_data[0].expiry_count > p_param.expiry_frame
    } {
        // The metadata read from bitstream is too old.
        pcm_dmx_reset(Some(self_), PCMDMX_RESET_BS_DATA);
    }
    let bs_meta_data = self_.bs_meta_data[p_param.frame_delay as usize & 1];
    // Maintain delay line
    let mut slot = p_param.frame_delay as usize;
    while slot > 0 {
        self_.bs_meta_data[slot & 1] = self_.bs_meta_data[(slot - 1) & 1];
        slot -= 1;
    }

    // - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - -
    if num_in_channels > num_out_channels {
        // Apply downmix
        let mut p_in_pcm: [Option<isize>; 8] = [None; 8];
        let mut p_out_pcm: [Option<isize>; 8] = [None; 8];
        let mut mix_factors = [[0 as FixpDmx; 8]; 8];
        let mut out_offset_table = [0u8; 8];
        let mut ch_cfg: i32 = 0;

        if num_in_channels > SIX_CHANNEL {
            // Get the type of the multipurpose channels (a slot without input channel reads
            // outside channelType in the reference; the port reads ACT_NONE)
            let multi_purpose_ch_type =
                [LEFT_MULTIPRPS_CHANNEL, RIGHT_MULTIPRPS_CHANNEL].map(|s| {
                    channel_type.get(in_offset_table[s] as usize).copied().unwrap_or(ACT_NONE)
                });

            // Check if the input configuration is one defined in the standard.
            ch_cfg = match in_ch_mode {
                CH_MODE_5_0_2_1 => {
                    // chCfg 7 || 14
                    // Further analyse the input config to distinguish the two CH_MODE_5_0_2_1
                    // configs.
                    if multi_purpose_ch_type[0] == ACT_FRONT_TOP
                        && multi_purpose_ch_type[1] == ACT_FRONT_TOP
                    {
                        14
                    } else {
                        7
                    }
                }
                CH_MODE_3_0_3_1 => 11, // chCfg 11
                CH_MODE_3_0_4_1 => 12, // chCfg 12
                _ => 0,                // Not a known config
            };
        }

        // Set this stages output stride and channel mode:
        let out_stride: isize = if f_interleaved != 0 { num_out_channels as isize } else { 1 };
        let out_ch_mode = OUT_CH_MODE_TABLE[num_out_channels as usize];
        fdk_assert!(out_ch_mode != CH_MODE_UNDEFINED, "FDK_ASSERT(outChMode != CH_MODE_UNDEFINED)");

        // Get channel description and channel mapping for the desired output configuration.
        get_channel_description(
            out_ch_mode,
            map_descr,
            channel_type,
            channel_indices,
            &mut out_offset_table,
        );
        // Now there is no way back because we modified the channel configuration!

        // Create the DMX matrix
        error_status = get_mix_factors(
            if ch_cfg > 0 { 1 } else { 0 },
            if ch_cfg > 0 { ch_cfg } else { in_ch_mode },
            out_ch_mode,
            &p_param,
            &bs_meta_data,
            &mut mix_factors,
            &mut dmx_scale,
        );
        // No fatal errors can occur here. The function is designed to always return a valid
        // matrix. The error code is used to signal configurations and matrices that are not
        // conform to any standard.

        // Determine the final scaling
        let byp_scale = f_min(dmx_max_scale, dmx_scale);
        *p_dmx_out_scale += byp_scale;
        dmx_scale -= byp_scale;

        {
            // Set channel pointer for input. Remove empty cols.
            let mut map = [0usize; 8];
            let mut ch = 0usize;
            for in_ch in 0..8 {
                if (in_offset_table[in_ch] as i32) < (num_in_channels as u8) as i32 {
                    p_in_pcm[ch] = Some(in_offset_table[in_ch] as isize * offset);
                    map[ch] = in_ch;
                    ch += 1;
                }
            }
            while ch < 8 {
                map[ch] = ch;
                ch += 1;
            }

            // Remove unused cols from factor matrix
            for in_ch in 0..num_in_channels as usize {
                if in_ch != map[in_ch] {
                    for out_ch in 0..8 {
                        mix_factors[out_ch][in_ch] = mix_factors[out_ch][map[in_ch]];
                    }
                }
            }

            // Set channel pointer for output. Remove empty cols.
            ch = 0;
            for out_ch in 0..8 {
                if (out_offset_table[out_ch] as i32) < (num_out_channels as u8) as i32 {
                    p_out_pcm[ch] = Some(out_offset_table[out_ch] as isize * offset);
                    map[ch] = out_ch;
                    ch += 1;
                }
            }
            while ch < 8 {
                map[ch] = ch;
                ch += 1;
            }

            // Remove unused rows from factor matrix
            for out_ch in 0..num_out_channels as usize {
                if out_ch != map[out_ch] {
                    mix_factors[out_ch] = mix_factors[map[out_ch]];
                }
            }
        }

        // Sample processing loop
        for _sample in 0..frame_size {
            let mut t_in = [0 as DmxPcm; 8];
            let mut t_out = [0 as FixpDbl; 8];

            // Preload all input samples
            for in_ch in 0..num_in_channels as usize {
                if let Some(p) = p_in_pcm[in_ch].as_mut() {
                    t_in[in_ch] = rd(pcm, *p);
                    *p += in_stride;
                } else {
                    t_in[in_ch] = 0;
                }
            }
            // Apply downmix coefficients to input samples and accumulate for output
            for out_ch in 0..num_out_channels as usize {
                for in_ch in 0..num_in_channels as usize {
                    t_out[out_ch] =
                        t_out[out_ch].wrapping_add(f_mult(t_in[in_ch], mix_factors[out_ch][in_ch]));
                }
                // Write sample (a missing output channel is skipped)
                if let Some(p) = p_out_pcm[out_ch].as_mut() {
                    wr(
                        pcm,
                        *p,
                        saturate_shift(
                            t_out[out_ch],
                            DFRACT_BITS - DMX_PCM_BITS - dmx_scale,
                            DMX_PCM_BITS,
                        ),
                    );
                    *p += out_stride;
                }
            }
        }

        // Update the number of output channels
        *n_channels = num_out_channels;
    }
    // - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - -
    else if num_in_channels < num_out_channels {
        // Apply rudimentary upmix
        // Set up channel pointer
        let mut out_offset_table = [0u8; 8];

        // FIRST STAGE
        //   Create a stereo/dual channel signal
        if num_in_channels == ONE_CHANNEL {
            // Set this stages output stride and channel mode:
            let out_stride: isize = if f_interleaved != 0 { TWO_CHANNEL as isize } else { 1 };
            let out_ch_mode = OUT_CH_MODE_TABLE[TWO_CHANNEL as usize];

            // Get channel description and channel mapping for this stages number of output
            // channels (always STEREO).
            get_channel_description(
                out_ch_mode,
                map_descr,
                channel_type,
                channel_indices,
                &mut out_offset_table,
            );
            // Now there is no way back because we modified the channel configuration!

            // Set input channel pointer. The first channel is always at index 0.
            // (Considering input mapping could lead to a invalid pointer here if the channel is
            // not declared to be a front channel.)
            let mut p_in_c = frame_size_m1 * in_stride;

            // Set output channel pointer (for this stage).
            let mut p_out_lf =
                out_offset_table[LEFT_FRONT_CHANNEL] as isize * offset + frame_size_m1 * out_stride;
            let mut p_out_rf = out_offset_table[RIGHT_FRONT_CHANNEL] as isize * offset
                + frame_size_m1 * out_stride;

            // 1/0 input:
            for _sample in 0..frame_size {
                // L' = C;  R' = C;
                let v = rd(pcm, p_in_c);
                wr(pcm, p_out_rf, v);
                wr(pcm, p_out_lf, v);

                p_in_c -= in_stride;
                p_out_lf -= out_stride;
                p_out_rf -= out_stride;
            }

            // Prepare for next stage:
            in_stride = out_stride;
            in_ch_mode = out_ch_mode;
            in_offset_table = out_offset_table;
        }

        // SECOND STAGE
        //   Extend with zero channels to achieved the desired number of output channels.
        if num_out_channels > TWO_CHANNEL {
            let mut p_in: [Option<isize>; 8] = [None; 8];
            let mut p_out: [Option<isize>; 8] = [None; 8];
            let mut in_ch_types = [ACT_NONE; 8];
            let mut in_ch_indices = [0u8; 8];
            let mut num_ch_per_grp = [[0u8; 4]; 2];
            let mut n_content_ch: usize = 0; // Number of channels with content
            let mut n_empty_ch: usize = 0; // Number of channels with content
            let mut is_compatible = true;
            const IN: usize = 0;
            const OUT: usize = 1;

            // Do not change the signalling which is the channel types and indices. Just reorder
            // and add channels. So first save the input signalling.
            let n_in = num_in_channels as usize;
            in_ch_types[..n_in].copy_from_slice(&channel_type[..n_in]);
            in_ch_indices[..n_in].copy_from_slice(&channel_indices[..n_in]);

            // Set this stages output stride and channel mode:
            let out_stride: isize = if f_interleaved != 0 { num_out_channels as isize } else { 1 };
            let out_ch_mode = OUT_CH_MODE_TABLE[num_out_channels as usize];
            fdk_assert!(
                out_ch_mode != CH_MODE_UNDEFINED,
                "FDK_ASSERT(outChMode != CH_MODE_UNDEFINED)"
            );

            // Check if input channel config can be easily mapped to the desired output config.
            for ch_grp in 0..4usize {
                num_ch_per_grp[IN][ch_grp] = ((in_ch_mode >> (ch_grp * 4)) & 0xF) as u8;
                num_ch_per_grp[OUT][ch_grp] = ((out_ch_mode >> (ch_grp * 4)) & 0xF) as u8;

                if num_ch_per_grp[IN][ch_grp] > num_ch_per_grp[OUT][ch_grp] {
                    is_compatible = false;
                    break;
                }
            }

            if is_compatible {
                // Get new channel description and channel mapping for the desired output channel
                // mode.
                get_channel_description(
                    out_ch_mode,
                    map_descr,
                    channel_type,
                    channel_indices,
                    &mut out_offset_table,
                );
                // If the input config has a back center channel but the output config has not,
                // copy it to left and right (if available).
                if num_ch_per_grp[IN][CH_GROUP_REAR] % 2 != 0
                    && num_ch_per_grp[OUT][CH_GROUP_REAR] % 2 == 0
                {
                    if num_ch_per_grp[IN][CH_GROUP_REAR] == 1 {
                        in_offset_table[RIGHT_REAR_CHANNEL] = in_offset_table[LEFT_REAR_CHANNEL];
                    } else if num_ch_per_grp[IN][CH_GROUP_REAR] == 3 {
                        in_offset_table[RIGHT_MULTIPRPS_CHANNEL] =
                            in_offset_table[LEFT_MULTIPRPS_CHANNEL];
                    }
                }
            } else {
                // Just copy and extend the original config
                out_offset_table = in_offset_table;
            }

            // Set I/O channel pointer.
            // Note: The following assignment algorithm clears the channel offset tables. Thus
            // they can not be used afterwards.
            for ch in 0..8 {
                if out_offset_table[ch] < 255 && in_offset_table[ch] < 255 {
                    // Set I/O pointer:
                    p_in[n_content_ch] =
                        Some(in_offset_table[ch] as isize * offset + frame_size_m1 * in_stride);
                    p_out[n_content_ch] =
                        Some(out_offset_table[ch] as isize * offset + frame_size_m1 * out_stride);
                    // Update signalling
                    let t =
                        in_ch_types.get(in_offset_table[ch] as usize).copied().unwrap_or(ACT_NONE);
                    let i = in_ch_indices.get(in_offset_table[ch] as usize).copied().unwrap_or(0);
                    set_at(channel_type, out_offset_table[ch] as usize, t);
                    set_at(channel_indices, out_offset_table[ch] as usize, i);
                    in_offset_table[ch] = 255;
                    out_offset_table[ch] = 255;
                    n_content_ch += 1;
                }
            }
            if is_compatible {
                // Assign the remaining input channels.
                // This is just a safety appliance. We should never need it.
                for ch in 0..8 {
                    if in_offset_table[ch] < 255 {
                        let mut out_ch = 0usize;
                        while out_ch < 8 {
                            if out_offset_table[out_ch] < 255 {
                                break;
                            }
                            out_ch += 1;
                        }
                        if out_ch >= 8 {
                            fdk_assert!(false, "FDK_ASSERT(0)");
                            break;
                        }
                        // Set I/O pointer:
                        p_in[n_content_ch] =
                            Some(in_offset_table[ch] as isize * offset + frame_size_m1 * in_stride);
                        p_out[n_content_ch] = Some(
                            out_offset_table[out_ch] as isize * offset + frame_size_m1 * out_stride,
                        );
                        // Update signalling
                        fdk_assert!(
                            (in_offset_table[out_ch] as i32) < num_in_channels,
                            "FDK_ASSERT(inOffsetTable[outCh] < numInChannels)"
                        );
                        fdk_assert!(
                            (out_offset_table[out_ch] as i32) < num_out_channels,
                            "FDK_ASSERT(outOffsetTable[outCh] < numOutChannels)"
                        );
                        let t = in_ch_types
                            .get(in_offset_table[ch] as usize)
                            .copied()
                            .unwrap_or(ACT_NONE);
                        let i =
                            in_ch_indices.get(in_offset_table[ch] as usize).copied().unwrap_or(0);
                        set_at(channel_type, out_offset_table[out_ch] as usize, t);
                        set_at(channel_indices, out_offset_table[out_ch] as usize, i);
                        in_offset_table[ch] = 255;
                        out_offset_table[out_ch] = 255;
                        n_content_ch += 1;
                    }
                }
                // Set the remaining output channel pointer
                for ch in 0..8 {
                    if out_offset_table[ch] < 255 {
                        if let Some(p) = p_out.get_mut(n_content_ch + n_empty_ch) {
                            *p = Some(
                                out_offset_table[ch] as isize * offset + frame_size_m1 * out_stride,
                            );
                        }
                        // Expand output signalling
                        set_at(channel_type, out_offset_table[ch] as usize, ACT_NONE);
                        set_at(channel_indices, out_offset_table[ch] as usize, n_empty_ch as u8);
                        out_offset_table[ch] = 255;
                        n_empty_ch += 1;
                    }
                }
            } else {
                // Set the remaining output channel pointer
                for ch in n_content_ch..num_out_channels as usize {
                    p_out[ch] = Some(ch as isize * offset + frame_size_m1 * out_stride);
                    // Expand output signalling
                    channel_type[ch] = ACT_NONE;
                    channel_indices[ch] = n_empty_ch as u8;
                    n_empty_ch += 1;
                }
            }

            // First copy the channels that have signal
            for _sample in 0..frame_size {
                let mut t_in = [0 as DmxPcm; 8];
                // Read all channel samples
                for ch in 0..n_content_ch {
                    if let Some(p) = p_in[ch].as_mut() {
                        t_in[ch] = rd(pcm, *p);
                        *p -= in_stride;
                    }
                }
                // Write all channel samples
                for ch in 0..n_content_ch {
                    if let Some(p) = p_out[ch].as_mut() {
                        wr(pcm, *p, t_in[ch]);
                        *p -= out_stride;
                    }
                }
            }

            // Clear all the other channels (a missing output channel is skipped)
            for _sample in 0..frame_size {
                for ch in n_content_ch..num_out_channels as usize {
                    if let Some(p) = p_out[ch].as_mut() {
                        wr(pcm, *p, 0);
                        *p -= out_stride;
                    }
                }
            }
        }

        // update the number of output channels
        *n_channels = num_out_channels;
    }
    // - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - -
    else if num_in_channels == num_out_channels {
        // Don't need to change the channel description here

        if num_in_channels == 2 {
            // Set up channel pointer
            let out_stride: isize;
            if f_interleaved != 0 {
                in_stride = num_in_channels as isize;
                out_stride = 2; // fixed !!! (below stereo is donwmixed to mono if required
                offset = 1; // Channel specific offset factor
            } else {
                in_stride = 1;
                out_stride = 1;
                offset = frame_size as i32 as isize; // Channel specific offset factor
            }

            // Set input channel pointer
            let mut p_in_l = in_offset_table[LEFT_FRONT_CHANNEL] as isize * offset;
            let mut p_in_r = in_offset_table[RIGHT_FRONT_CHANNEL] as isize * offset;

            // Set output channel pointer (same as input)
            let mut p_out_l = p_in_l;
            let mut p_out_r = p_in_r;

            // Set downmix levels:
            let flev: FixpDmx = fl2fxconst_dmx(0.70710678);
            // 2/0 input:
            match dual_channel_mode {
                CH1_MODE => {
                    // L' = 0.707 * Ch1;  R' = 0.707 * Ch1
                    for _sample in 0..frame_size {
                        let v = saturate_right_shift(
                            f_mult(rd(pcm, p_in_l), flev),
                            DFRACT_BITS - DMX_PCM_BITS,
                            DMX_PCM_BITS,
                        );
                        wr(pcm, p_out_r, v);
                        wr(pcm, p_out_l, v);

                        p_in_l += in_stride;
                        p_out_l += out_stride;
                        p_out_r += out_stride;
                    }
                }
                CH2_MODE => {
                    // L' = 0.707 * Ch2;  R' = 0.707 * Ch2
                    for _sample in 0..frame_size {
                        let v = saturate_right_shift(
                            f_mult(rd(pcm, p_in_r), flev),
                            DFRACT_BITS - DMX_PCM_BITS,
                            DMX_PCM_BITS,
                        );
                        wr(pcm, p_out_r, v);
                        wr(pcm, p_out_l, v);

                        p_in_r += in_stride;
                        p_out_l += out_stride;
                        p_out_r += out_stride;
                    }
                }
                MIXED_MODE => {
                    // L' = 0.5*Ch1 + 0.5*Ch2;  R' = 0.5*Ch1 + 0.5*Ch2
                    for _sample in 0..frame_size {
                        let v = (rd(pcm, p_in_l) >> 1) + (rd(pcm, p_in_r) >> 1);
                        wr(pcm, p_out_r, v);
                        wr(pcm, p_out_l, v);

                        p_in_l += in_stride;
                        p_in_r += in_stride;
                        p_out_l += out_stride;
                        p_out_r += out_stride;
                    }
                }
                // STEREO_MODE and default: nothing to do
                _ => {}
            }
        }
        // default: nothing to do
    }

    error_status
}

/// `pcmDmx_Close(pSelf)`: closes an instance of the PCM downmix module.
pub fn pcm_dmx_close(p_self: Option<&mut Option<HandlePcmDownmix>>) -> PcmdmxError {
    let Some(p_self) = p_self else {
        return PCMDMX_INVALID_HANDLE;
    };

    // FreePcmDmxInstance(pSelf)
    *p_self = None;

    PCMDMX_OK
}

/// `pcmDmx_GetLibInfo(info)`: enters the library information of the downmix module into the
/// first free entry of `info` (an array of `FDK_MODULE_LAST` entries), then the FDK tools'.
///
/// A slice shorter than `FDK_MODULE_LAST` entries without a free entry is treated as full.
pub fn pcm_dmx_get_lib_info(info: Option<&mut [LibInfo]>) -> PcmdmxError {
    let Some(info) = info else {
        return PCMDMX_INVALID_ARGUMENT;
    };

    // Search for next free tab
    let mut i = 0usize;
    while i < FDK_MODULE_LAST as usize {
        match info.get(i) {
            Some(e) if e.module_id == FDK_NONE => break,
            Some(_) => i += 1,
            None => return PCMDMX_INVALID_ARGUMENT,
        }
    }
    if i == FDK_MODULE_LAST as usize {
        return PCMDMX_INVALID_ARGUMENT;
    }

    // Add the library info
    let e = &mut info[i];
    e.module_id = FDK_PCMDMX;
    e.version = lib_version(PCMUTIL_LIB_VL0, PCMUTIL_LIB_VL1, PCMUTIL_LIB_VL2);
    lib_version_string(e);
    e.build_date = PCMUTIL_LIB_BUILD_DATE;
    e.build_time = PCMUTIL_LIB_BUILD_TIME;
    e.title = PCMDMX_LIB_TITLE;

    // Set flags
    e.flags = CAPF_DMX_BLIND // At least blind downmixing is possible
        | CAPF_DMX_PCE // Guided downmix with data from MPEG-2/4 Program Config Elements (PCE).
        | CAPF_DMX_ARIB // PCE guided downmix with slightly different equations and levels.
        | CAPF_DMX_DVB // Guided downmix with data from DVB ancillary data fields.
        | CAPF_DMX_CH_EXP // Simple upmixing by dublicating channels or adding zero channels.
        | CAPF_DMX_6_CH
        | CAPF_DMX_8_CH;

    // Add lib info for FDK tools (if not yet done).
    fdk_tools_get_lib_info(Some(info));

    PCMDMX_OK
}
