// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libSYS/include/FDK_audio.h.

//! Port of `FDK_audio.h`: global audio types and constants.
//!
//! The enums are `i32` type aliases with constants, because the C++ code stores, compares and
//! computes with them as integers. The values of the enumerators and
//! of the `#define` constants were generated from the C++ header with the compiler and are
//! compared with the reference by the differential tests.
//!
//! The structures `CODER_CONFIG`, `LIB_INFO` and `FDK_bufDescr` are ported as [`CoderConfig`],
//! [`LibInfo`] and [`FdkBufDescr`].

use core::ffi::c_void;

// ---------------------------------------------------------------------------------------------
// Enumerations
// ---------------------------------------------------------------------------------------------

/// `FILE_FORMAT`
pub type FileFormat = i32;
pub const FF_UNKNOWN: FileFormat = -1;
pub const FF_RAW: FileFormat = 0;
pub const FF_MP4_3GPP: FileFormat = 3;
pub const FF_MP4_MP4F: FileFormat = 4;
pub const FF_RAWPACKETS: FileFormat = 5;

/// `TRANSPORT_TYPE`
pub type TransportType = i32;
pub const TT_UNKNOWN: TransportType = -1;
pub const TT_MP4_RAW: TransportType = 0;
pub const TT_MP4_ADIF: TransportType = 1;
pub const TT_MP4_ADTS: TransportType = 2;
pub const TT_MP4_LATM_MCP1: TransportType = 6;
pub const TT_MP4_LATM_MCP0: TransportType = 7;
pub const TT_MP4_LOAS: TransportType = 10;
pub const TT_DRM: TransportType = 12;

/// `AUDIO_OBJECT_TYPE`
pub type AudioObjectType = i32;
pub const AOT_NONE: AudioObjectType = -1;
pub const AOT_NULL_OBJECT: AudioObjectType = 0;
pub const AOT_AAC_MAIN: AudioObjectType = 1;
pub const AOT_AAC_LC: AudioObjectType = 2;
pub const AOT_AAC_SSR: AudioObjectType = 3;
pub const AOT_AAC_LTP: AudioObjectType = 4;
pub const AOT_SBR: AudioObjectType = 5;
pub const AOT_AAC_SCAL: AudioObjectType = 6;
pub const AOT_TWIN_VQ: AudioObjectType = 7;
pub const AOT_CELP: AudioObjectType = 8;
pub const AOT_HVXC: AudioObjectType = 9;
pub const AOT_RSVD_10: AudioObjectType = 10;
pub const AOT_RSVD_11: AudioObjectType = 11;
pub const AOT_TTSI: AudioObjectType = 12;
pub const AOT_MAIN_SYNTH: AudioObjectType = 13;
pub const AOT_WAV_TAB_SYNTH: AudioObjectType = 14;
pub const AOT_GEN_MIDI: AudioObjectType = 15;
pub const AOT_ALG_SYNTH_AUD_FX: AudioObjectType = 16;
pub const AOT_ER_AAC_LC: AudioObjectType = 17;
pub const AOT_RSVD_18: AudioObjectType = 18;
pub const AOT_ER_AAC_LTP: AudioObjectType = 19;
pub const AOT_ER_AAC_SCAL: AudioObjectType = 20;
pub const AOT_ER_TWIN_VQ: AudioObjectType = 21;
pub const AOT_ER_BSAC: AudioObjectType = 22;
pub const AOT_ER_AAC_LD: AudioObjectType = 23;
pub const AOT_ER_CELP: AudioObjectType = 24;
pub const AOT_ER_HVXC: AudioObjectType = 25;
pub const AOT_ER_HILN: AudioObjectType = 26;
pub const AOT_ER_PARA: AudioObjectType = 27;
pub const AOT_RSVD_28: AudioObjectType = 28;
pub const AOT_PS: AudioObjectType = 29;
pub const AOT_MPEGS: AudioObjectType = 30;
pub const AOT_ESCAPE: AudioObjectType = 31;
pub const AOT_MP3ONMP4_L1: AudioObjectType = 32;
pub const AOT_MP3ONMP4_L2: AudioObjectType = 33;
pub const AOT_MP3ONMP4_L3: AudioObjectType = 34;
pub const AOT_RSVD_35: AudioObjectType = 35;
pub const AOT_RSVD_36: AudioObjectType = 36;
pub const AOT_AAC_SLS: AudioObjectType = 37;
pub const AOT_SLS: AudioObjectType = 38;
pub const AOT_ER_AAC_ELD: AudioObjectType = 39;
pub const AOT_USAC: AudioObjectType = 42;
pub const AOT_SAOC: AudioObjectType = 43;
pub const AOT_LD_MPEGS: AudioObjectType = 44;
pub const AOT_MP2_AAC_LC: AudioObjectType = 129;
pub const AOT_MP2_SBR: AudioObjectType = 132;
pub const AOT_DRM_AAC: AudioObjectType = 143;
pub const AOT_DRM_SBR: AudioObjectType = 144;
pub const AOT_DRM_MPEG_PS: AudioObjectType = 145;
pub const AOT_DRM_SURROUND: AudioObjectType = 146;
pub const AOT_DRM_USAC: AudioObjectType = 147;

/// `CHANNEL_MODE`
pub type ChannelMode = i32;
pub const MODE_INVALID: ChannelMode = -1;
pub const MODE_UNKNOWN: ChannelMode = 0;
pub const MODE_1: ChannelMode = 1;
pub const MODE_2: ChannelMode = 2;
pub const MODE_1_2: ChannelMode = 3;
pub const MODE_1_2_1: ChannelMode = 4;
pub const MODE_1_2_2: ChannelMode = 5;
pub const MODE_1_2_2_1: ChannelMode = 6;
pub const MODE_1_2_2_2_1: ChannelMode = 7;
pub const MODE_6_1: ChannelMode = 11;
pub const MODE_7_1_BACK: ChannelMode = 12;
pub const MODE_7_1_TOP_FRONT: ChannelMode = 14;
pub const MODE_7_1_REAR_SURROUND: ChannelMode = 33;
pub const MODE_7_1_FRONT_CENTER: ChannelMode = 34;
pub const MODE_212: ChannelMode = 128;

/// `AUDIO_CHANNEL_TYPE`
pub type AudioChannelType = i32;
pub const ACT_NONE: AudioChannelType = 0;
pub const ACT_FRONT: AudioChannelType = 1;
pub const ACT_SIDE: AudioChannelType = 2;
pub const ACT_BACK: AudioChannelType = 3;
pub const ACT_LFE: AudioChannelType = 4;
pub const ACT_TOP: AudioChannelType = 16;
pub const ACT_FRONT_TOP: AudioChannelType = 17;
pub const ACT_SIDE_TOP: AudioChannelType = 18;
pub const ACT_BACK_TOP: AudioChannelType = 19;
pub const ACT_BOTTOM: AudioChannelType = 32;
pub const ACT_FRONT_BOTTOM: AudioChannelType = 33;
pub const ACT_SIDE_BOTTOM: AudioChannelType = 34;
pub const ACT_BACK_BOTTOM: AudioChannelType = 35;

/// `SBR_PS_SIGNALING`
pub type SbrPsSignaling = i32;
pub const SIG_UNKNOWN: SbrPsSignaling = -1;
pub const SIG_IMPLICIT: SbrPsSignaling = 0;
pub const SIG_EXPLICIT_BW_COMPATIBLE: SbrPsSignaling = 1;
pub const SIG_EXPLICIT_HIERARCHICAL: SbrPsSignaling = 2;

/// `MP4_ELEMENT_ID`
pub type Mp4ElementId = i32;
pub const ID_NONE: Mp4ElementId = -1;
pub const ID_SCE: Mp4ElementId = 0;
pub const ID_CPE: Mp4ElementId = 1;
pub const ID_CCE: Mp4ElementId = 2;
pub const ID_LFE: Mp4ElementId = 3;
pub const ID_DSE: Mp4ElementId = 4;
pub const ID_PCE: Mp4ElementId = 5;
pub const ID_FIL: Mp4ElementId = 6;
pub const ID_END: Mp4ElementId = 7;
pub const ID_EXT: Mp4ElementId = 8;
pub const ID_SCAL: Mp4ElementId = 9;
pub const ID_USAC_SCE: Mp4ElementId = 16;
pub const ID_USAC_CPE: Mp4ElementId = 17;
pub const ID_USAC_LFE: Mp4ElementId = 18;
pub const ID_USAC_EXT: Mp4ElementId = 19;
pub const ID_USAC_END: Mp4ElementId = 20;
pub const ID_LAST: Mp4ElementId = 21;

/// `CONFIG_EXT_ID`
pub type ConfigExtId = i32;
pub const ID_CONFIG_EXT_FILL: ConfigExtId = 0;
pub const ID_CONFIG_EXT_DOWNMIX: ConfigExtId = 1;
pub const ID_CONFIG_EXT_LOUDNESS_INFO: ConfigExtId = 2;
pub const ID_CONFIG_EXT_AUDIOSCENE_INFO: ConfigExtId = 3;
pub const ID_CONFIG_EXT_HOA_MATRIX: ConfigExtId = 4;
pub const ID_CONFIG_EXT_SIG_GROUP_INFO: ConfigExtId = 6;

/// `EXT_PAYLOAD_TYPE`
pub type ExtPayloadType = i32;
pub const EXT_FIL: ExtPayloadType = 0;
pub const EXT_FILL_DATA: ExtPayloadType = 1;
pub const EXT_DATA_ELEMENT: ExtPayloadType = 2;
pub const EXT_DATA_LENGTH: ExtPayloadType = 3;
pub const EXT_UNI_DRC: ExtPayloadType = 4;
pub const EXT_LDSAC_DATA: ExtPayloadType = 9;
pub const EXT_SAOC_DATA: ExtPayloadType = 10;
pub const EXT_DYNAMIC_RANGE: ExtPayloadType = 11;
pub const EXT_SAC_DATA: ExtPayloadType = 12;
pub const EXT_SBR_DATA: ExtPayloadType = 13;
pub const EXT_SBR_DATA_CRC: ExtPayloadType = 14;

/// `USAC_EXT_ELEMENT_TYPE`
pub type UsacExtElementType = i32;
pub const ID_EXT_ELE_FILL: UsacExtElementType = 0;
pub const ID_EXT_ELE_MPEGS: UsacExtElementType = 1;
pub const ID_EXT_ELE_SAOC: UsacExtElementType = 2;
pub const ID_EXT_ELE_AUDIOPREROLL: UsacExtElementType = 3;
pub const ID_EXT_ELE_UNI_DRC: UsacExtElementType = 4;
pub const ID_EXT_ELE_OBJ_METADATA: UsacExtElementType = 5;
pub const ID_EXT_ELE_SAOC_3D: UsacExtElementType = 6;
pub const ID_EXT_ELE_HOA: UsacExtElementType = 7;
pub const ID_EXT_ELE_FMT_CNVRTR: UsacExtElementType = 8;
pub const ID_EXT_ELE_MCT: UsacExtElementType = 9;
pub const ID_EXT_ELE_ENHANCED_OBJ_METADATA: UsacExtElementType = 13;
pub const ID_EXT_ELE_VR_METADATA: UsacExtElementType = 129;
pub const ID_EXT_ELE_UNKNOWN: UsacExtElementType = 255;

/// `TP_CONFIG_TYPE`
pub type TpConfigType = i32;
pub const TC_NOTHING: TpConfigType = 0;
pub const TC_RAW_ADTS: TpConfigType = 2;
pub const TC_RAW_LATM_MCP1: TpConfigType = 6;
pub const TC_RAW_SDC: TpConfigType = 21;

/// `FDK_MODULE_ID`
pub type FdkModuleId = i32;
pub const FDK_NONE: FdkModuleId = 0;
pub const FDK_TOOLS: FdkModuleId = 1;
pub const FDK_SYSLIB: FdkModuleId = 2;
pub const FDK_AACDEC: FdkModuleId = 3;
pub const FDK_AACENC: FdkModuleId = 4;
pub const FDK_SBRDEC: FdkModuleId = 5;
pub const FDK_SBRENC: FdkModuleId = 6;
pub const FDK_TPDEC: FdkModuleId = 7;
pub const FDK_TPENC: FdkModuleId = 8;
pub const FDK_MPSDEC: FdkModuleId = 9;
pub const FDK_MPEGFILEREAD: FdkModuleId = 10;
pub const FDK_MPEGFILEWRITE: FdkModuleId = 11;
pub const FDK_PCMDMX: FdkModuleId = 31;
pub const FDK_MPSENC: FdkModuleId = 34;
pub const FDK_TDLIMIT: FdkModuleId = 35;
pub const FDK_UNIDRCDEC: FdkModuleId = 38;
pub const FDK_MODULE_LAST: FdkModuleId = 39;

// ---------------------------------------------------------------------------------------------
// #define constants
// ---------------------------------------------------------------------------------------------

pub const AC_ER_VCB11: u32 = 0x00000001;
pub const AC_ER_RVLC: u32 = 0x00000002;
pub const AC_ER_HCR: u32 = 0x00000004;
pub const AC_SCALABLE: u32 = 0x00000008;
pub const AC_ELD: u32 = 0x00000010;
pub const AC_LD: u32 = 0x00000020;
pub const AC_ER: u32 = 0x00000040;
pub const AC_BSAC: u32 = 0x00000080;
pub const AC_USAC: u32 = 0x00000100;
pub const AC_RSV603DA: u32 = 0x00000200;
pub const AC_HDAAC: u32 = 0x00000400;
pub const AC_RSVD50: u32 = 0x00004000;
pub const AC_SBR_PRESENT: u32 = 0x00008000;
pub const AC_SBRCRC: u32 = 0x00010000;
pub const AC_PS_PRESENT: u32 = 0x00020000;
pub const AC_MPS_PRESENT: u32 = 0x00040000;
pub const AC_DRM: u32 = 0x00080000;
pub const AC_INDEP: u32 = 0x00100000;
pub const AC_MPEGD_RES: u32 = 0x00200000;
pub const AC_SAOC_PRESENT: u32 = 0x00400000;
pub const AC_DAB: u32 = 0x00800000;
pub const AC_ELD_DOWNSCALE: u32 = 0x01000000;
pub const AC_LD_MPS: u32 = 0x02000000;
pub const AC_DRC_PRESENT: u32 = 0x04000000;
pub const AC_USAC_SCFGI3: u32 = 0x08000000;
pub const AC_EL_USAC_TW: u32 = 0x00000001;
pub const AC_EL_USAC_NOISE: u32 = 0x00000002;
pub const AC_EL_USAC_ITES: u32 = 0x00000004;
pub const AC_EL_USAC_PVC: u32 = 0x00000008;
pub const AC_EL_USAC_MPS212: u32 = 0x00000010;
pub const AC_EL_USAC_LFE: u32 = 0x00000020;
pub const AC_EL_USAC_CP_POSSIBLE: u32 = 0x00000040;
pub const AC_EL_ENHANCED_NOISE: u32 = 0x00000080;
pub const AC_EL_IGF_AFTER_TNS: u32 = 0x00000100;
pub const AC_EL_IGF_INDEP_TILING: u32 = 0x00000200;
pub const AC_EL_IGF_USE_ENF: u32 = 0x00000400;
pub const AC_EL_FULLBANDLPD: u32 = 0x00000800;
pub const AC_EL_LPDSTEREOIDX: u32 = 0x00001000;
pub const AC_EL_LFE: u32 = 0x00002000;
pub const AC_CM_DET_CFG_CHANGE: u8 = 0x00000001;
pub const AC_CM_ALLOC_MEM: u8 = 0x00000002;
pub const CC_MPEG_ID: u32 = 0x00100000;
pub const CC_IS_BASELAYER: u32 = 0x00200000;
pub const CC_PROTECTION: u32 = 0x00400000;
pub const CC_SBR: u32 = 0x00800000;
pub const CC_SBRCRC: u32 = 0x00010000;
pub const CC_SAC: u32 = 0x00020000;
pub const CC_RVLC: u32 = 0x01000000;
pub const CC_VCB11: u32 = 0x02000000;
pub const CC_HCR: u32 = 0x04000000;
pub const CC_PSEUDO_SURROUND: u32 = 0x08000000;
pub const CC_USAC_NOISE: u32 = 0x10000000;
pub const CC_USAC_TW: u32 = 0x20000000;
pub const CC_USAC_HBE: u32 = 0x40000000;
pub const CAPF_AAC_LC: u32 = 0x00000001;
pub const CAPF_ER_AAC_LD: u32 = 0x00000002;
pub const CAPF_ER_AAC_SCAL: u32 = 0x00000004;
pub const CAPF_ER_AAC_LC: u32 = 0x00000008;
pub const CAPF_AAC_480: u32 = 0x00000010;
pub const CAPF_AAC_512: u32 = 0x00000020;
pub const CAPF_AAC_960: u32 = 0x00000040;
pub const CAPF_AAC_1024: u32 = 0x00000080;
pub const CAPF_AAC_HCR: u32 = 0x00000100;
pub const CAPF_AAC_VCB11: u32 = 0x00000200;
pub const CAPF_AAC_RVLC: u32 = 0x00000400;
pub const CAPF_AAC_MPEG4: u32 = 0x00000800;
pub const CAPF_AAC_DRC: u32 = 0x00001000;
pub const CAPF_AAC_CONCEALMENT: u32 = 0x00002000;
pub const CAPF_AAC_DRM_BSFORMAT: u32 = 0x00004000;
pub const CAPF_ER_AAC_ELD: u32 = 0x00008000;
pub const CAPF_ER_AAC_BSAC: u32 = 0x00010000;
pub const CAPF_AAC_ELD_DOWNSCALE: u32 = 0x00040000;
pub const CAPF_AAC_USAC_LP: u32 = 0x00100000;
pub const CAPF_AAC_USAC: u32 = 0x00200000;
pub const CAPF_ER_AAC_ELDV2: u32 = 0x00800000;
pub const CAPF_AAC_UNIDRC: u32 = 0x01000000;
pub const CAPF_ADTS: u32 = 0x00000001;
pub const CAPF_ADIF: u32 = 0x00000002;
pub const CAPF_LATM: u32 = 0x00000004;
pub const CAPF_LOAS: u32 = 0x00000008;
pub const CAPF_RAWPACKETS: u32 = 0x00000010;
pub const CAPF_DRM: u32 = 0x00000020;
pub const CAPF_RSVD50: u32 = 0x00000040;
pub const CAPF_SBR_LP: u32 = 0x00000001;
pub const CAPF_SBR_HQ: u32 = 0x00000002;
pub const CAPF_SBR_DRM_BS: u32 = 0x00000004;
pub const CAPF_SBR_CONCEALMENT: u32 = 0x00000008;
pub const CAPF_SBR_DRC: u32 = 0x00000010;
pub const CAPF_SBR_PS_MPEG: u32 = 0x00000020;
pub const CAPF_SBR_PS_DRM: u32 = 0x00000040;
pub const CAPF_SBR_ELD_DOWNSCALE: u32 = 0x00000080;
pub const CAPF_SBR_HBEHQ: u32 = 0x00000100;
pub const CAPF_DMX_BLIND: u32 = 0x00000001;
pub const CAPF_DMX_PCE: u32 = 0x00000002;
pub const CAPF_DMX_ARIB: u32 = 0x00000004;
pub const CAPF_DMX_DVB: u32 = 0x00000008;
pub const CAPF_DMX_CH_EXP: u32 = 0x00000010;
pub const CAPF_DMX_6_CH: u32 = 0x00000020;
pub const CAPF_DMX_8_CH: u32 = 0x00000040;
pub const CAPF_DMX_24_CH: u32 = 0x00000080;
pub const CAPF_LIMITER: u32 = 0x00002000;
pub const CAPF_MPS_STD: u32 = 0x00000001;
pub const CAPF_MPS_LD: u32 = 0x00000002;
pub const CAPF_MPS_USAC: u32 = 0x00000004;
pub const CAPF_MPS_HQ: u32 = 0x00000010;
pub const CAPF_MPS_LP: u32 = 0x00000020;
pub const CAPF_MPS_BLIND: u32 = 0x00000040;
pub const CAPF_MPS_BINAURAL: u32 = 0x00000080;
pub const CAPF_MPS_2CH_OUT: u32 = 0x00000100;
pub const CAPF_MPS_6CH_OUT: u32 = 0x00000200;
pub const CAPF_MPS_8CH_OUT: u32 = 0x00000400;
pub const CAPF_MPS_1CH_IN: u32 = 0x00001000;
pub const CAPF_MPS_2CH_IN: u32 = 0x00002000;
pub const CAPF_MPS_6CH_IN: u32 = 0x00004000;
pub const FDK_BUF_TYPE_MASK_IO: u32 = 0xC0000000;
pub const FDK_BUF_TYPE_MASK_DESCR: u32 = 0x003F0000;
pub const FDK_BUF_TYPE_MASK_ID: u32 = 0x000000FF;
pub const FDK_BUF_TYPE_INPUT: u32 = 0x40000000;
pub const FDK_BUF_TYPE_OUTPUT: u32 = 0x80000000;
pub const FDK_BUF_TYPE_PCM_DATA: u32 = 0x00010000;
pub const FDK_BUF_TYPE_ANC_DATA: u32 = 0x00020000;
pub const FDK_BUF_TYPE_BS_DATA: u32 = 0x00040000;
pub const USAC_ID_BIT: i32 = 16;
pub const EXT_ID_BITS: i32 = 4;

// ---------------------------------------------------------------------------------------------
// Macros
// ---------------------------------------------------------------------------------------------

/// `TT_IS_PACKET(x)`
#[inline]
pub const fn tt_is_packet(x: TransportType) -> bool {
    x == TT_MP4_RAW || x == TT_DRM || x == TT_MP4_LATM_MCP0 || x == TT_MP4_LATM_MCP1
}

/// `CAN_DO_PS(aot)`
#[inline]
pub const fn can_do_ps(aot: AudioObjectType) -> bool {
    aot == AOT_AAC_LC || aot == AOT_SBR || aot == AOT_PS || aot == AOT_ER_BSAC || aot == AOT_DRM_AAC
}

/// `IS_USAC(aot)`
#[inline]
pub const fn is_usac(aot: AudioObjectType) -> bool {
    aot == AOT_USAC
}

/// `IS_LOWDELAY(aot)`
#[inline]
pub const fn is_lowdelay(aot: AudioObjectType) -> bool {
    aot == AOT_ER_AAC_LD || aot == AOT_ER_AAC_ELD
}

/// `IS_CHANNEL_ELEMENT(elementId)`
#[inline]
pub const fn is_channel_element(element_id: Mp4ElementId) -> bool {
    element_id == ID_SCE
        || element_id == ID_CPE
        || element_id == ID_LFE
        || element_id == ID_USAC_SCE
        || element_id == ID_USAC_CPE
        || element_id == ID_USAC_LFE
}

/// `IS_MP4_CHANNEL_ELEMENT(elementId)`
#[inline]
pub const fn is_mp4_channel_element(element_id: Mp4ElementId) -> bool {
    element_id == ID_SCE || element_id == ID_CPE || element_id == ID_LFE
}

/// `IS_USAC_CHANNEL_ELEMENT(elementId)`
#[inline]
pub const fn is_usac_channel_element(element_id: Mp4ElementId) -> bool {
    element_id == ID_USAC_SCE || element_id == ID_USAC_CPE || element_id == ID_USAC_LFE
}

// ---------------------------------------------------------------------------------------------
// CODER_CONFIG
// ---------------------------------------------------------------------------------------------

/// `CODER_CONFIG`: generic audio coder configuration structure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CoderConfig {
    /// `aot`: Audio Object Type (AOT).
    pub aot: AudioObjectType,
    /// `extAOT`: Extension Audio Object Type (SBR).
    pub ext_aot: AudioObjectType,
    /// `channelMode`: Channel mode.
    pub channel_mode: ChannelMode,
    /// `channelConfigZero`: Use channel config zero + pce although a standard channel config
    /// could be signaled.
    pub channel_config_zero: u8,
    /// `samplingRate`: Sampling rate.
    pub sampling_rate: i32,
    /// `extSamplingRate`: Extended samplerate (SBR).
    pub ext_sampling_rate: i32,
    /// `downscaleSamplingRate`: Downscale sampling rate (ELD downscaled mode).
    pub downscale_sampling_rate: i32,
    /// `bitRate`: Average bitrate.
    pub bit_rate: i32,
    /// `samplesPerFrame`: Number of PCM samples per codec frame and audio channel.
    pub samples_per_frame: i32,
    /// `noChannels`: Number of audio channels.
    pub no_channels: i32,
    /// `bitsFrame`
    pub bits_frame: i32,
    /// `nSubFrames`: Amount of encoder subframes. 1 means no subframing.
    pub n_sub_frames: i32,
    /// `BSACnumOfSubFrame`: The number of the sub-frames which are grouped and transmitted in a
    /// super-frame (BSAC).
    pub bsac_num_of_sub_frame: i32,
    /// `BSAClayerLength`: The average length of the large-step layers in bytes (BSAC).
    pub bsac_layer_length: i32,
    /// `flags`
    pub flags: u32,
    /// `matrixMixdownA`: Matrix mixdown index to put into PCE. Default value 0 means no mixdown
    /// coefficient, valid values are 1-4 which correspond to matrix_mixdown_idx 0-3.
    pub matrix_mixdown_a: u8,
    /// `headerPeriod`: Frame period for sending in band configuration buffers in the transport
    /// layer.
    pub header_period: u8,
    /// `stereoConfigIndex`: USAC MPS stereo mode
    pub stereo_config_index: u8,
    /// `sbrMode`: USAC SBR mode
    pub sbr_mode: u8,
    /// `sbrSignaling`: 0: implicit signaling, 1: backwards compatible explicit signaling, 2:
    /// hierarcical explicit signaling
    pub sbr_signaling: SbrPsSignaling,
    /// `rawConfig`: raw codec specific config as bit stream
    pub raw_config: [u8; 64],
    /// `rawConfigBits`: Size of rawConfig in bits
    pub raw_config_bits: i32,
    /// `sbrPresent`
    pub sbr_present: u8,
    /// `psPresent`
    pub ps_present: u8,
}

impl Default for CoderConfig {
    /// All fields zero, like `FDKmemclear(cc, sizeof(CODER_CONFIG))`.
    fn default() -> Self {
        Self {
            aot: 0,
            ext_aot: 0,
            channel_mode: 0,
            channel_config_zero: 0,
            sampling_rate: 0,
            ext_sampling_rate: 0,
            downscale_sampling_rate: 0,
            bit_rate: 0,
            samples_per_frame: 0,
            no_channels: 0,
            bits_frame: 0,
            n_sub_frames: 0,
            bsac_num_of_sub_frame: 0,
            bsac_layer_length: 0,
            flags: 0,
            matrix_mixdown_a: 0,
            header_period: 0,
            stereo_config_index: 0,
            sbr_mode: 0,
            sbr_signaling: 0,
            raw_config: [0; 64],
            raw_config_bits: 0,
            sbr_present: 0,
            ps_present: 0,
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Library versioning
// ---------------------------------------------------------------------------------------------

/// `LIB_VERSION(lev0, lev1, lev2)`: one numeric version from the three levels.
#[inline]
pub const fn lib_version(lev0: i32, lev1: i32, lev2: i32) -> i32 {
    (((lev0 << 24) as u32 & 0xff00_0000)
        | ((lev1 << 16) as u32 & 0x00ff_0000)
        | ((lev2 << 8) as u32 & 0x0000_ff00)) as i32
}

/// `LIB_INFO`: library information.
///
/// The strings are `const char*` in C++; the port keeps them as `&'static str` (they are string
/// literals in fdk-aac). The C ABI layer converts them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LibInfo {
    /// `title`
    pub title: &'static str,
    /// `build_date`
    pub build_date: &'static str,
    /// `build_time`
    pub build_time: &'static str,
    /// `module_id`
    pub module_id: FdkModuleId,
    /// `version`
    pub version: i32,
    /// `flags`
    pub flags: u32,
    /// `versionStr`: NUL-terminated
    pub version_str: [u8; 32],
}

impl Default for LibInfo {
    fn default() -> Self {
        Self {
            title: "",
            build_date: "",
            build_time: "",
            module_id: FDK_NONE,
            version: 0,
            flags: 0,
            version_str: [0; 32],
        }
    }
}

/// `LIB_VERSION_STRING(info)`: writes `"%d.%d.%d"` of the three version levels to
/// `info.version_str`.
pub fn lib_version_string(info: &mut LibInfo) {
    let v = info.version;
    let mut s = [0u8; 32];
    let mut n = 0;
    for (k, shift) in [24, 16, 8].into_iter().enumerate() {
        if k > 0 {
            s[n] = b'.';
            n += 1;
        }
        let level = (v >> shift) & 0xff;
        let digits = if level >= 100 {
            3
        } else if level >= 10 {
            2
        } else {
            1
        };
        let mut x = level;
        for i in (0..digits).rev() {
            s[n + i] = b'0' + (x % 10) as u8;
            x /= 10;
        }
        n += digits;
    }
    s[n] = 0;
    info.version_str[..=n].copy_from_slice(&s[..=n]);
}

/// `FDKinitLibInfo(info)`: marks the first `FDK_MODULE_LAST` entries as free (`FDK_NONE`).
#[inline]
pub fn fdk_init_lib_info(info: &mut [LibInfo]) {
    for e in info.iter_mut().take(FDK_MODULE_LAST as usize) {
        e.module_id = FDK_NONE;
    }
}

/// `FDKlibInfo_getCapabilities(info, module_id)`: the flags of the entry of `module_id`, 0 if
/// there is none among the first `FDK_MODULE_LAST` entries.
#[inline]
pub fn fdk_lib_info_get_capabilities(info: &[LibInfo], module_id: FdkModuleId) -> u32 {
    for e in info.iter().take(FDK_MODULE_LAST as usize) {
        if e.module_id == module_id {
            return e.flags;
        }
    }
    0
}

/// `FDKlibInfo_lookup(info, module_id)`: the index of the first free entry, -1 if `module_id`
/// is already registered before it or if all `FDK_MODULE_LAST` entries are used.
#[inline]
pub fn fdk_lib_info_lookup(info: &[LibInfo], module_id: FdkModuleId) -> i32 {
    let mut i = 0;
    while i < FDK_MODULE_LAST {
        // The C++ code reads FDK_MODULE_LAST entries; a shorter slice ends the search.
        let Some(e) = info.get(i as usize) else {
            return -1;
        };
        if e.module_id == module_id {
            return -1;
        }
        if e.module_id == FDK_NONE {
            break;
        }
        i += 1;
    }
    if i == FDK_MODULE_LAST {
        return -1;
    }
    i
}

// ---------------------------------------------------------------------------------------------
// Buffer description
// ---------------------------------------------------------------------------------------------

/// `FDK_bufDescr`: I/O buffer descriptor of the C API (raw pointers, as in C++).
#[derive(Clone, Copy, Debug)]
pub struct FdkBufDescr {
    /// `ppBase`: array of buffer base addresses (NULL for buffer requirement info).
    pub pp_base: *mut *mut c_void,
    /// `pBufSize`: array of the number of elements that fit into each buffer.
    pub p_buf_size: *mut u32,
    /// `pEleSize`: array of the element size of each buffer in bytes.
    pub p_ele_size: *mut u32,
    /// `pBufType`: array of bit fields describing each buffer (`FDK_BUF_TYPE_*`).
    pub p_buf_type: *mut u32,
    /// `numBufs`: total number of buffers.
    pub num_bufs: u32,
}

impl Default for FdkBufDescr {
    fn default() -> Self {
        Self {
            pp_base: core::ptr::null_mut(),
            p_buf_size: core::ptr::null_mut(),
            p_ele_size: core::ptr::null_mut(),
            p_buf_type: core::ptr::null_mut(),
            num_bufs: 0,
        }
    }
}
