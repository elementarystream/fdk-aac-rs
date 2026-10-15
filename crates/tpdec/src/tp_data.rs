// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libMpegTPDec/include/tp_data.h.

//! Port of `tp_data.h`: the configuration structures shared by the transport decoder, the
//! transport encoder and the AAC decoder and encoder (`CSAudioSpecificConfig`,
//! `CProgramConfig`, `CSUsacConfig`, ...) and the transport decoder callbacks.
//!
//! `libMpegTPEnc/include/tp_data.h` differs from this file only in a comment; `fdk-aac-rs-tpenc`
//! re-exports these types.
//!
//! # Unions
//!
//! `CSAudioSpecificConfig::m_sc` is a union of the GA, ELD and USAC specific configurations in C.
//! The port stores the three members separately ([`CSAudioSpecificConfigSc`]). This is exact:
//! every writer clears the whole structure first (`AudioSpecificConfig_Init()`), and every
//! reader of the library reads the member that belongs to the audio object type that was parsed.
//! The only cross-member read, `m_sc.m_usacConfig.element[0].extElement
//! .usacExtElementHasAudioPreRoll` for a non-USAC configuration in
//! `CLatmDemux_ReadAudioMuxElement()`, reads byte 33 of the union, which lies behind the GA
//! (32 bytes) and ELD (12 bytes) members and is therefore 0 in C as well.
//!
//! # Callbacks
//!
//! The C callbacks (`CSTpCallBacks`: a function pointer plus an opaque `void *` per callback)
//! are expressed by the trait [`TpDecCallbacks`]; see the module documentation of
//! [`crate::tpdec_lib`] for the design. [`CSTpCallBacks`] is the C structure itself (function
//! pointers and user data pointers); it implements the trait.

use core::ffi::c_void;

use fdk_aac_rs_fdk::audio::{AudioObjectType, Mp4ElementId, UsacExtElementType};
use fdk_aac_rs_fdk::bitstream::FdkBitStream;

// ---------------------------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------------------------

/// `TP_USAC_MAX_SPEAKERS`
pub const TP_USAC_MAX_SPEAKERS: i32 = 24;
/// `TP_USAC_MAX_EXT_ELEMENTS`
pub const TP_USAC_MAX_EXT_ELEMENTS: usize = 24;
/// `TP_USAC_MAX_ELEMENTS`
pub const TP_USAC_MAX_ELEMENTS: usize = 24 + TP_USAC_MAX_EXT_ELEMENTS;
/// `TP_USAC_MAX_CONFIG_LEN`: next power of two of maximum of `escapedValue(hBs, 4, 4, 8)` in
/// `AudioPreRoll()` (285).
pub const TP_USAC_MAX_CONFIG_LEN: usize = 512;
/// `TPDEC_USAC_NUM_CONFIG_CHANGE_FRAMES`: number of frames for config change in USAC.
pub const TPDEC_USAC_NUM_CONFIG_CHANGE_FRAMES: i8 = 1;

pub const TPDEC_FLUSH_OFF: u8 = 0;
pub const TPDEC_RSV60_CFG_CHANGE_ATSC_FLUSH_ON: u8 = 1;
pub const TPDEC_RSV60_DASH_IPF_ATSC_FLUSH_ON: u8 = 2;
pub const TPDEC_USAC_DASH_IPF_FLUSH_ON: u8 = 3;

pub const TPDEC_BUILD_UP_OFF: u8 = 0;
pub const TPDEC_RSV60_BUILD_UP_ON: u8 = 1;
pub const TPDEC_RSV60_BUILD_UP_ON_IN_BAND: u8 = 2;
pub const TPDEC_USAC_BUILD_UP_ON: u8 = 3;
pub const TPDEC_RSV60_BUILD_UP_IDLE: u8 = 4;
pub const TPDEC_RSV60_BUILD_UP_IDLE_IN_BAND: u8 = 5;

// ---------------------------------------------------------------------------------------------
// CProgramConfig
// ---------------------------------------------------------------------------------------------

/// `PC_FSB_CHANNELS_MAX`: front/side/back channels.
pub const PC_FSB_CHANNELS_MAX: usize = 16;
/// `PC_LFE_CHANNELS_MAX`
pub const PC_LFE_CHANNELS_MAX: usize = 4;
/// `PC_ASSOCDATA_MAX`
pub const PC_ASSOCDATA_MAX: usize = 8;
/// `PC_CCEL_MAX`: CC elements.
pub const PC_CCEL_MAX: usize = 16;
/// `PC_COMMENTLENGTH`
pub const PC_COMMENTLENGTH: usize = 256;
/// `PC_NUM_HEIGHT_LAYER`
pub const PC_NUM_HEIGHT_LAYER: usize = 3;

/// `CProgramConfig`: program config element (ISO/IEC 14496-3 4.4.1.1 Table 4.2).
///
/// All members are `UCHAR`, so the C structure has no padding; `#[repr(C)]` gives the port the
/// same layout. Accesses of `CProgramConfig_Check()` that continue from an array into the
/// following members are reproduced from the C layout with [`CProgramConfig::as_bytes`].
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CProgramConfig {
    // PCE bitstream elements:
    pub element_instance_tag: u8,
    pub profile: u8,
    pub sampling_frequency_index: u8,
    pub num_front_channel_elements: u8,
    pub num_side_channel_elements: u8,
    pub num_back_channel_elements: u8,
    pub num_lfe_channel_elements: u8,
    pub num_assoc_data_elements: u8,
    pub num_valid_cc_elements: u8,

    pub mono_mixdown_present: u8,
    pub mono_mixdown_element_number: u8,

    pub stereo_mixdown_present: u8,
    pub stereo_mixdown_element_number: u8,

    pub matrix_mixdown_index_present: u8,
    pub matrix_mixdown_index: u8,
    pub pseudo_surround_enable: u8,

    pub front_element_is_cpe: [u8; PC_FSB_CHANNELS_MAX],
    pub front_element_tag_select: [u8; PC_FSB_CHANNELS_MAX],
    pub front_element_height_info: [u8; PC_FSB_CHANNELS_MAX],

    pub side_element_is_cpe: [u8; PC_FSB_CHANNELS_MAX],
    pub side_element_tag_select: [u8; PC_FSB_CHANNELS_MAX],
    pub side_element_height_info: [u8; PC_FSB_CHANNELS_MAX],

    pub back_element_is_cpe: [u8; PC_FSB_CHANNELS_MAX],
    pub back_element_tag_select: [u8; PC_FSB_CHANNELS_MAX],
    pub back_element_height_info: [u8; PC_FSB_CHANNELS_MAX],

    pub lfe_element_tag_select: [u8; PC_LFE_CHANNELS_MAX],

    pub assoc_data_element_tag_select: [u8; PC_ASSOCDATA_MAX],

    pub cc_element_is_ind_sw: [u8; PC_CCEL_MAX],
    pub valid_cc_element_tag_select: [u8; PC_CCEL_MAX],

    pub comment_field_bytes: u8,
    pub comment: [u8; PC_COMMENTLENGTH],

    // Helper variables for administration:
    /// Flag showing if PCE has been read successfully.
    pub is_valid: u8,
    /// Amount of audio channels summing all channel elements including LFEs.
    pub num_channels: u8,
    /// Amount of audio channels summing only SCEs and CPEs.
    pub num_effective_channels: u8,
    pub el_counter: u8,
}

/// `sizeof(CProgramConfig)`
pub const CPROGRAMCONFIG_SIZE: usize = 465;
const _: () = assert!(core::mem::size_of::<CProgramConfig>() == CPROGRAMCONFIG_SIZE);

/// Byte offset of `FrontElementIsCpe` in `CProgramConfig`.
pub const PCE_OFFSET_FRONT_ELEMENT_IS_CPE: usize = 16;
/// Byte offset of `SideElementIsCpe` in `CProgramConfig`.
pub const PCE_OFFSET_SIDE_ELEMENT_IS_CPE: usize = 16 + 3 * PC_FSB_CHANNELS_MAX;
/// Byte offset of `BackElementIsCpe` in `CProgramConfig`.
pub const PCE_OFFSET_BACK_ELEMENT_IS_CPE: usize = 16 + 6 * PC_FSB_CHANNELS_MAX;

impl CProgramConfig {
    /// A zeroed structure (`FDKmemclear()`).
    pub const fn zeroed() -> Self {
        Self {
            element_instance_tag: 0,
            profile: 0,
            sampling_frequency_index: 0,
            num_front_channel_elements: 0,
            num_side_channel_elements: 0,
            num_back_channel_elements: 0,
            num_lfe_channel_elements: 0,
            num_assoc_data_elements: 0,
            num_valid_cc_elements: 0,
            mono_mixdown_present: 0,
            mono_mixdown_element_number: 0,
            stereo_mixdown_present: 0,
            stereo_mixdown_element_number: 0,
            matrix_mixdown_index_present: 0,
            matrix_mixdown_index: 0,
            pseudo_surround_enable: 0,
            front_element_is_cpe: [0; PC_FSB_CHANNELS_MAX],
            front_element_tag_select: [0; PC_FSB_CHANNELS_MAX],
            front_element_height_info: [0; PC_FSB_CHANNELS_MAX],
            side_element_is_cpe: [0; PC_FSB_CHANNELS_MAX],
            side_element_tag_select: [0; PC_FSB_CHANNELS_MAX],
            side_element_height_info: [0; PC_FSB_CHANNELS_MAX],
            back_element_is_cpe: [0; PC_FSB_CHANNELS_MAX],
            back_element_tag_select: [0; PC_FSB_CHANNELS_MAX],
            back_element_height_info: [0; PC_FSB_CHANNELS_MAX],
            lfe_element_tag_select: [0; PC_LFE_CHANNELS_MAX],
            assoc_data_element_tag_select: [0; PC_ASSOCDATA_MAX],
            cc_element_is_ind_sw: [0; PC_CCEL_MAX],
            valid_cc_element_tag_select: [0; PC_CCEL_MAX],
            comment_field_bytes: 0,
            comment: [0; PC_COMMENTLENGTH],
            is_valid: 0,
            num_channels: 0,
            num_effective_channels: 0,
            el_counter: 0,
        }
    }

    /// The structure as the bytes of its C layout.
    #[inline]
    pub fn as_bytes(&self) -> &[u8; CPROGRAMCONFIG_SIZE] {
        // SAFETY: `CProgramConfig` is `#[repr(C)]` and consists of `u8` fields and arrays only,
        // so it has size CPROGRAMCONFIG_SIZE (asserted above), alignment 1 and no padding.
        unsafe { &*(self as *const Self as *const [u8; CPROGRAMCONFIG_SIZE]) }
    }
}

impl Default for CProgramConfig {
    fn default() -> Self {
        Self::zeroed()
    }
}

// ---------------------------------------------------------------------------------------------
// AudioSpecificConfig
// ---------------------------------------------------------------------------------------------

/// `TP_ASC_EXTENSION_ID`
pub type TpAscExtensionId = i32;
pub const ASCEXT_UNKOWN: TpAscExtensionId = -1;
pub const ASCEXT_SBR: TpAscExtensionId = 0x2b7;
pub const ASCEXT_PS: TpAscExtensionId = 0x548;
pub const ASCEXT_MPS: TpAscExtensionId = 0x76a;
pub const ASCEXT_SAOC: TpAscExtensionId = 0x7cb;
pub const ASCEXT_LDMPS: TpAscExtensionId = 0x7cc;

/// `CSGaSpecificConfig`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CSGaSpecificConfig {
    pub m_frame_length_flag: u32,
    pub m_depends_on_core_coder: u32,
    pub m_core_coder_delay: u32,

    pub m_extension_flag: u32,
    pub m_extension_flag3: u32,

    pub m_layer: u32,
    pub m_num_of_sub_frame: u32,
    pub m_layer_length: u32,
}

/// `ASC_ELD_EXT_TYPE`
pub type AscEldExtType = u32;
/// Termination tag.
pub const ELDEXT_TERM: AscEldExtType = 0x0;
/// SAOC config.
pub const ELDEXT_SAOC: AscEldExtType = 0x1;
/// LD MPEG Surround config.
pub const ELDEXT_LDSAC: AscEldExtType = 0x2;
/// ELD sample rate adaptation.
pub const ELDEXT_DOWNSCALEINFO: AscEldExtType = 0x3;

/// `CSEldSpecificConfig`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CSEldSpecificConfig {
    pub m_frame_length_flag: u8,

    pub m_sbr_present_flag: u8,
    /// Use LD-MPS QMF in SBR to achive time alignment.
    pub m_use_ld_qmf_time_align: u8,
    pub m_sbr_sampling_rate: u8,
    pub m_sbr_crc_flag: u8,
    pub m_downscaled_sampling_frequency: u32,
}

/// `CSUsacExtElementConfig`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CSUsacExtElementConfig {
    pub usac_ext_element_type: UsacExtElementType,
    pub usac_ext_element_config_length: u16,
    pub usac_ext_element_default_length: u16,
    pub usac_ext_element_payload_frag: u8,
    pub usac_ext_element_has_audio_pre_roll: u8,
}

/// `CSUsacElementConfig`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CSUsacElementConfig {
    pub usac_element_type: Mp4ElementId,
    pub m_noise_filling: u8,
    pub m_harmonic_sbr: u8,
    pub m_inter_tes: u8,
    pub m_pvc: u8,
    pub m_stereo_config_index: u8,
    pub ext_element: CSUsacExtElementConfig,
}

/// `CSUsacConfig`
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CSUsacConfig {
    pub m_frame_length_flag: u8,
    pub m_core_sbr_frame_length_index: u8,
    pub m_sbr_ratio_index: u8,
    /// Number of audio channels signaled in `UsacDecoderConfig()` / `rsv603daDecoderConfig()`
    /// via `numElements` and `usacElementType`.
    pub m_n_usac_channels: u8,
    pub m_channel_configuration_index: u8,
    pub m_usac_num_elements: u32,
    pub element: [CSUsacElementConfig; TP_USAC_MAX_ELEMENTS],

    pub num_audio_channels: u8,
    pub m_usac_config_extension_present: u8,
    pub element_length_present: u8,
    pub usac_config: [u8; TP_USAC_MAX_CONFIG_LEN],
    pub usac_config_bits: u16,
}

impl CSUsacConfig {
    /// A zeroed structure.
    pub const fn zeroed() -> Self {
        const E: CSUsacElementConfig = CSUsacElementConfig {
            usac_element_type: 0,
            m_noise_filling: 0,
            m_harmonic_sbr: 0,
            m_inter_tes: 0,
            m_pvc: 0,
            m_stereo_config_index: 0,
            ext_element: CSUsacExtElementConfig {
                usac_ext_element_type: 0,
                usac_ext_element_config_length: 0,
                usac_ext_element_default_length: 0,
                usac_ext_element_payload_frag: 0,
                usac_ext_element_has_audio_pre_roll: 0,
            },
        };
        Self {
            m_frame_length_flag: 0,
            m_core_sbr_frame_length_index: 0,
            m_sbr_ratio_index: 0,
            m_n_usac_channels: 0,
            m_channel_configuration_index: 0,
            m_usac_num_elements: 0,
            element: [E; TP_USAC_MAX_ELEMENTS],
            num_audio_channels: 0,
            m_usac_config_extension_present: 0,
            element_length_present: 0,
            usac_config: [0; TP_USAC_MAX_CONFIG_LEN],
            usac_config_bits: 0,
        }
    }
}

impl Default for CSUsacConfig {
    fn default() -> Self {
        Self::zeroed()
    }
}

/// The specific configuration of `CSAudioSpecificConfig` (`m_sc`, a union in C; see the module
/// documentation for why separate storage is exact).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CSAudioSpecificConfigSc {
    /// General audio specific configuration.
    pub m_ga_specific_config: CSGaSpecificConfig,
    /// ELD specific configuration.
    pub m_eld_specific_config: CSEldSpecificConfig,
    /// USAC specific configuration.
    pub m_usac_config: CSUsacConfig,
}

/// `CSAudioSpecificConfig`: audio configuration struct, suitable for encoder and decoder
/// configuration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CSAudioSpecificConfig {
    /// XYZ specific data.
    pub m_sc: CSAudioSpecificConfigSc,

    // Common ASC parameters
    /// Program configuration.
    pub m_progr_config_element: CProgramConfig,

    /// Audio Object Type.
    pub m_aot: AudioObjectType,
    /// Samplerate.
    pub m_sampling_frequency: u32,
    /// Amount of samples per frame.
    pub m_samples_per_frame: u32,
    pub m_direct_mapping: u32,

    /// Audio object type.
    pub m_extension_audio_object_type: AudioObjectType,
    /// Samplerate.
    pub m_extension_sampling_frequency: u32,

    /// Channel configuration index.
    pub m_channel_configuration: i8,

    /// Error protection index.
    pub m_ep_config: i8,
    /// aacSectionDataResilienceFlag
    pub m_vcb11_flag: i8,
    /// aacScalefactorDataResilienceFlag
    pub m_rvlc_flag: i8,
    /// aacSpectralDataResilienceFlag
    pub m_hcr_flag: i8,

    /// Flag indicating the presence of SBR data in the bitstream.
    pub m_sbr_present_flag: i8,
    /// Flag indicating the presence of parametric stereo data in the bitstream.
    pub m_ps_present_flag: i8,
    /// Samplerate index.
    pub m_sampling_frequency_index: u8,
    /// Samplerate index.
    pub m_extension_sampling_frequency_index: u8,
    /// Channel configuration index.
    pub m_extension_channel_configuration: i8,

    /// The flag indicates if the callback shall work in memory allocation mode or in config
    /// change detection mode.
    pub config_mode: u8,
    /// Set if at least one aac config parameter has changed that requires a memory
    /// reconfiguration, otherwise cleared.
    pub aac_config_changed: u8,
    /// Set if at least one sbr config parameter has changed that requires a memory
    /// reconfiguration, otherwise cleared.
    pub sbr_config_changed: u8,
    /// Set if at least one sac config parameter has changed that requires a memory
    /// reconfiguration, otherwise cleared.
    pub sac_config_changed: u8,

    /// Configuration stored as bitstream.
    pub config: [u8; TP_USAC_MAX_CONFIG_LEN],
    /// Configuration length in bits.
    pub config_bits: u32,
}

impl CSAudioSpecificConfig {
    /// A zeroed structure (`FDKmemclear()`; also the content of the `calloc()`ed transport
    /// decoder).
    pub const fn zeroed() -> Self {
        Self {
            m_sc: CSAudioSpecificConfigSc {
                m_ga_specific_config: CSGaSpecificConfig {
                    m_frame_length_flag: 0,
                    m_depends_on_core_coder: 0,
                    m_core_coder_delay: 0,
                    m_extension_flag: 0,
                    m_extension_flag3: 0,
                    m_layer: 0,
                    m_num_of_sub_frame: 0,
                    m_layer_length: 0,
                },
                m_eld_specific_config: CSEldSpecificConfig {
                    m_frame_length_flag: 0,
                    m_sbr_present_flag: 0,
                    m_use_ld_qmf_time_align: 0,
                    m_sbr_sampling_rate: 0,
                    m_sbr_crc_flag: 0,
                    m_downscaled_sampling_frequency: 0,
                },
                m_usac_config: CSUsacConfig::zeroed(),
            },
            m_progr_config_element: CProgramConfig::zeroed(),
            m_aot: 0,
            m_sampling_frequency: 0,
            m_samples_per_frame: 0,
            m_direct_mapping: 0,
            m_extension_audio_object_type: 0,
            m_extension_sampling_frequency: 0,
            m_channel_configuration: 0,
            m_ep_config: 0,
            m_vcb11_flag: 0,
            m_rvlc_flag: 0,
            m_hcr_flag: 0,
            m_sbr_present_flag: 0,
            m_ps_present_flag: 0,
            m_sampling_frequency_index: 0,
            m_extension_sampling_frequency_index: 0,
            m_extension_channel_configuration: 0,
            config_mode: 0,
            aac_config_changed: 0,
            sbr_config_changed: 0,
            sac_config_changed: 0,
            config: [0; TP_USAC_MAX_CONFIG_LEN],
            config_bits: 0,
        }
    }
}

impl Default for CSAudioSpecificConfig {
    fn default() -> Self {
        Self::zeroed()
    }
}

/// `CCtrlCFGChange`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CCtrlCFGChange {
    /// Flush frame counter.
    pub flush_cnt: i8,
    /// Flag indicates flush mode: on|off.
    pub flush_status: u8,
    /// Build up frame counter.
    pub build_up_cnt: i8,
    /// Flag indicates build up mode: on|off.
    pub build_up_status: u8,
    /// Flag indicates that the config changed and the decoder needs to be initialized again via
    /// callback. Make sure that memory is freed before initialization.
    pub cfg_changed: u8,
    /// Flag indicates that the content changed i.e. a right truncation occured before.
    pub content_changed: u8,
    /// Flag indicates if config change has to be forced even if new config is the same.
    pub force_cfg_change: u8,
}

// ---------------------------------------------------------------------------------------------
// Callbacks
// ---------------------------------------------------------------------------------------------

/// The transport decoder callbacks (`CSTpCallBacks`), implemented by the AAC decoder.
///
/// Each method corresponds to one C callback; the `has_*` methods correspond to the checks of
/// the C code whether a callback pointer is `NULL` (only made for `cbSsc`, `cbSbr`, `cbUsac`
/// and `cbUniDrc`; `cbUpdateConfig`, `cbFreeMem` and `cbCtrlCFGChange` are called
/// unconditionally). The opaque `void *` user data of C is `self`.
pub trait TpDecCallbacks {
    /// `cbUpdateConfig(self, pAsc, configMode, configChanged)`: (re)configure the decoder for
    /// `asc`.
    ///
    /// `asc` is always an element of the `asc` array of the transport decoder
    /// (`&hTp->asc[layer]`, layer 0), which persists after the call (the C decoder keeps a
    /// pointer into it, `pUsacConfig[]`). In C, `configChanged` may point into `*pAsc`
    /// (`&asc->AacConfigChanged`); the Rust caller then passes a copy of that field and stores
    /// the result back after the call. The reference decoder does not read `AacConfigChanged`
    /// through `pAsc`, so the copy is exact.
    fn update_config(
        &mut self,
        asc: &CSAudioSpecificConfig,
        config_mode: u8,
        config_changed: &mut u8,
    ) -> i32;

    /// `cbFreeMem(self, pAsc)`: free the configuration dependent memory.
    fn free_mem(&mut self, asc: &CSAudioSpecificConfig) -> i32;

    /// `cbCtrlCFGChange(self, pCtrlCFGChangeStruct)`: config change (flush / build up) control.
    fn ctrl_cfg_change(&mut self, ctrl: &CCtrlCFGChange) -> i32;

    /// `cbSsc != NULL`
    fn has_ssc(&self) -> bool;
    /// `cbSsc(self, hBs, coreCodec, samplingRate, frameSize, numChannels, stereoConfigIndex,
    /// coreSbrFrameLengthIndex, configBytes, configMode, configChanged)`: parse a MPEG Surround
    /// SpatialSpecificConfig.
    #[allow(clippy::too_many_arguments)]
    fn ssc(
        &mut self,
        hbs: &mut FdkBitStream,
        core_codec: AudioObjectType,
        sampling_rate: i32,
        frame_size: i32,
        num_channels: i32,
        stereo_config_index: i32,
        core_sbr_frame_length_index: i32,
        config_bytes: i32,
        config_mode: u8,
        config_changed: &mut u8,
    ) -> i32;

    /// `cbSbr != NULL`
    fn has_sbr(&self) -> bool;
    /// `cbSbr(self, hBs, sampleRateIn, sampleRateOut, samplesPerFrame, coreCodec, elementID,
    /// elementIndex, harmonicSbr, stereoConfigIndex, configMode, configChanged,
    /// downscaleFactor)`: parse an SBR header.
    #[allow(clippy::too_many_arguments)]
    fn sbr(
        &mut self,
        hbs: &mut FdkBitStream,
        sample_rate_in: i32,
        sample_rate_out: i32,
        samples_per_frame: i32,
        core_codec: AudioObjectType,
        element_id: Mp4ElementId,
        element_index: i32,
        harmonic_sbr: u8,
        stereo_config_index: u8,
        config_mode: u8,
        config_changed: &mut u8,
        downscale_factor: i32,
    ) -> i32;

    /// `cbUsac != NULL`
    fn has_usac(&self) -> bool;
    /// `cbUsac(self, hBs)` (registered by the decoder, never called by the transport decoder).
    fn usac(&mut self, hbs: &mut FdkBitStream) -> i32;

    /// `cbUniDrc != NULL`
    fn has_uni_drc(&self) -> bool;
    /// `cbUniDrc(self, hBs, fullPayloadLength, payloadType, subStreamIndex, payloadStart,
    /// aot)`: parse a uniDrcConfig (`payload_type == 0`) or a loudnessInfoSet
    /// (`payload_type == 1`). `hbs == None` (`NULL`) clears the respective data.
    #[allow(clippy::too_many_arguments)]
    fn uni_drc(
        &mut self,
        hbs: Option<&mut FdkBitStream>,
        full_payload_length: i32,
        payload_type: i32,
        sub_stream_index: i32,
        payload_start: i32,
        aot: AudioObjectType,
    ) -> i32;
}

/// `cbUpdateConfig_t`
pub type CbUpdateConfig = fn(*mut c_void, &CSAudioSpecificConfig, u8, &mut u8) -> i32;
/// `cbFreeMem_t`
pub type CbFreeMem = fn(*mut c_void, &CSAudioSpecificConfig) -> i32;
/// `cbCtrlCFGChange_t`
pub type CbCtrlCfgChange = fn(*mut c_void, &CCtrlCFGChange) -> i32;
/// `cbSsc_t`
pub type CbSsc = fn(
    *mut c_void,
    &mut FdkBitStream,
    AudioObjectType,
    i32,
    i32,
    i32,
    i32,
    i32,
    i32,
    u8,
    &mut u8,
) -> i32;
/// `cbSbr_t`
pub type CbSbr = fn(
    *mut c_void,
    &mut FdkBitStream,
    i32,
    i32,
    i32,
    AudioObjectType,
    Mp4ElementId,
    i32,
    u8,
    u8,
    u8,
    &mut u8,
    i32,
) -> i32;
/// `cbUsac_t`
pub type CbUsac = fn(*mut c_void, &mut FdkBitStream) -> i32;
/// `cbUniDrc_t`
pub type CbUniDrc =
    fn(*mut c_void, Option<&mut FdkBitStream>, i32, i32, i32, i32, AudioObjectType) -> i32;

/// `CSTpCallBacks`: the callback function pointers and their user data pointers, as in C.
///
/// For a callback whose pointer is `None` where the reference has no defined result, the port
/// skips the call and uses the result 0.
#[derive(Clone, Copy, Debug)]
pub struct CSTpCallBacks {
    /// Function pointer for Config change notify callback.
    pub cb_update_config: Option<CbUpdateConfig>,
    /// User data pointer for Config change notify callback.
    pub cb_update_config_data: *mut c_void,
    /// Function pointer for free memory callback.
    pub cb_free_mem: Option<CbFreeMem>,
    /// User data pointer for free memory callback.
    pub cb_free_mem_data: *mut c_void,
    /// Function pointer for config change control callback.
    pub cb_ctrl_cfg_change: Option<CbCtrlCfgChange>,
    /// User data pointer for config change control callback.
    pub cb_ctrl_cfg_change_data: *mut c_void,
    /// Function pointer for SSC parser callback.
    pub cb_ssc: Option<CbSsc>,
    /// User data pointer for SSC parser callback.
    pub cb_ssc_data: *mut c_void,
    /// Function pointer for SBR header parser callback.
    pub cb_sbr: Option<CbSbr>,
    /// User data pointer for SBR header parser callback.
    pub cb_sbr_data: *mut c_void,
    pub cb_usac: Option<CbUsac>,
    pub cb_usac_data: *mut c_void,
    /// Function pointer for uniDrcConfig and loudnessInfoSet parser callback.
    pub cb_uni_drc: Option<CbUniDrc>,
    /// User data pointer for uniDrcConfig and loudnessInfoSet parser callback.
    pub cb_uni_drc_data: *mut c_void,
}

impl Default for CSTpCallBacks {
    fn default() -> Self {
        Self {
            cb_update_config: None,
            cb_update_config_data: core::ptr::null_mut(),
            cb_free_mem: None,
            cb_free_mem_data: core::ptr::null_mut(),
            cb_ctrl_cfg_change: None,
            cb_ctrl_cfg_change_data: core::ptr::null_mut(),
            cb_ssc: None,
            cb_ssc_data: core::ptr::null_mut(),
            cb_sbr: None,
            cb_sbr_data: core::ptr::null_mut(),
            cb_usac: None,
            cb_usac_data: core::ptr::null_mut(),
            cb_uni_drc: None,
            cb_uni_drc_data: core::ptr::null_mut(),
        }
    }
}

// SAFETY: the user data pointers are only handed to the callbacks; moving the structure to
// another thread is as safe as moving the objects they point to.
unsafe impl Send for CSTpCallBacks {}

impl TpDecCallbacks for CSTpCallBacks {
    fn update_config(
        &mut self,
        asc: &CSAudioSpecificConfig,
        config_mode: u8,
        config_changed: &mut u8,
    ) -> i32 {
        match self.cb_update_config {
            Some(f) => f(self.cb_update_config_data, asc, config_mode, config_changed),
            None => 0,
        }
    }

    fn free_mem(&mut self, asc: &CSAudioSpecificConfig) -> i32 {
        match self.cb_free_mem {
            Some(f) => f(self.cb_free_mem_data, asc),
            None => 0,
        }
    }

    fn ctrl_cfg_change(&mut self, ctrl: &CCtrlCFGChange) -> i32 {
        match self.cb_ctrl_cfg_change {
            Some(f) => f(self.cb_ctrl_cfg_change_data, ctrl),
            None => 0,
        }
    }

    fn has_ssc(&self) -> bool {
        self.cb_ssc.is_some()
    }

    fn ssc(
        &mut self,
        hbs: &mut FdkBitStream,
        core_codec: AudioObjectType,
        sampling_rate: i32,
        frame_size: i32,
        num_channels: i32,
        stereo_config_index: i32,
        core_sbr_frame_length_index: i32,
        config_bytes: i32,
        config_mode: u8,
        config_changed: &mut u8,
    ) -> i32 {
        match self.cb_ssc {
            Some(f) => f(
                self.cb_ssc_data,
                hbs,
                core_codec,
                sampling_rate,
                frame_size,
                num_channels,
                stereo_config_index,
                core_sbr_frame_length_index,
                config_bytes,
                config_mode,
                config_changed,
            ),
            None => 0,
        }
    }

    fn has_sbr(&self) -> bool {
        self.cb_sbr.is_some()
    }

    fn sbr(
        &mut self,
        hbs: &mut FdkBitStream,
        sample_rate_in: i32,
        sample_rate_out: i32,
        samples_per_frame: i32,
        core_codec: AudioObjectType,
        element_id: Mp4ElementId,
        element_index: i32,
        harmonic_sbr: u8,
        stereo_config_index: u8,
        config_mode: u8,
        config_changed: &mut u8,
        downscale_factor: i32,
    ) -> i32 {
        match self.cb_sbr {
            Some(f) => f(
                self.cb_sbr_data,
                hbs,
                sample_rate_in,
                sample_rate_out,
                samples_per_frame,
                core_codec,
                element_id,
                element_index,
                harmonic_sbr,
                stereo_config_index,
                config_mode,
                config_changed,
                downscale_factor,
            ),
            None => 0,
        }
    }

    fn has_usac(&self) -> bool {
        self.cb_usac.is_some()
    }

    fn usac(&mut self, hbs: &mut FdkBitStream) -> i32 {
        match self.cb_usac {
            Some(f) => f(self.cb_usac_data, hbs),
            None => 0,
        }
    }

    fn has_uni_drc(&self) -> bool {
        self.cb_uni_drc.is_some()
    }

    fn uni_drc(
        &mut self,
        hbs: Option<&mut FdkBitStream>,
        full_payload_length: i32,
        payload_type: i32,
        sub_stream_index: i32,
        payload_start: i32,
        aot: AudioObjectType,
    ) -> i32 {
        match self.cb_uni_drc {
            Some(f) => f(
                self.cb_uni_drc_data,
                hbs,
                full_payload_length,
                payload_type,
                sub_stream_index,
                payload_start,
                aot,
            ),
            None => 0,
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Tables and helpers
// ---------------------------------------------------------------------------------------------

/// `SamplingRateTable[]`
pub static SAMPLING_RATE_TABLE: [u32; 32] = [
    96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000, 7350, 0, 0,
    57600, 51200, 40000, 38400, 34150, 28800, 25600, 20000, 19200, 17075, 14400, 12800, 9600, 0, 0,
    0, 0,
];

/// `SamplingRateTable[idx]`; indices outside the table give 0.
#[inline]
pub fn sampling_rate_table(idx: usize) -> u32 {
    match SAMPLING_RATE_TABLE.get(idx) {
        Some(&v) => v,
        None => 0,
    }
}

/// `getSamplingRateIndex(samplingRate, nBits)`
///
/// The library calls it with `nBits == 4`. The port treats entries outside `SamplingRateTable`
/// as 0.
#[inline]
pub fn get_sampling_rate_index(sampling_rate: u32, n_bits: u32) -> i32 {
    let table_size: u32 = 1u32.wrapping_shl(n_bits).wrapping_sub(1);

    let mut sf_index: u32 = 0;
    while sf_index < table_size {
        if sampling_rate_table(sf_index as usize) == sampling_rate {
            break;
        }
        sf_index += 1;
    }

    if sf_index > table_size {
        return table_size.wrapping_sub(1) as i32;
    }

    sf_index as i32
}

/// `getNumberOfTotalChannels(channelConfig)`: the channel count of a channel configuration.
#[inline]
pub fn get_number_of_total_channels(channel_config: i32) -> i32 {
    match channel_config {
        1..=6 => channel_config,
        7 | 12 | 14 => 8,
        11 => 7,
        13 => 24,
        _ => 0,
    }
}

/// `getNumberOfEffectiveChannels(channelConfig)`
///
/// The C function indexes a local array of 16 entries; the library calls it with ADTS channel
/// configurations (0..7) only. Other values (no defined reference result) give 0.
#[inline]
pub fn get_number_of_effective_channels(channel_config: i32) -> i32 {
    // index: 0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15
    const N: [i32; 16] = [0, 1, 2, 3, 4, 5, 5, 7, 0, 0, 0, 6, 7, 22, 7, 0];
    match usize::try_from(channel_config).ok().and_then(|i| N.get(i)) {
        Some(&n) => n,
        None => 0,
    }
}
