// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACdec/src/aacdecoder.h, libAACdec/src/aacdecoder.cpp.

//! Port of `aacdecoder.h`: the AAC decoder core instance (`struct AAC_DECODER_INSTANCE`), its
//! configuration (`CAacDecoder_Init()`), the element loop (`CAacDecoder_DecodeFrame()`),
//! extension payloads, ancillary data and the program config element.
//!
//! # Memory (see also [`crate::channelinfo`])
//!
//! | C | Port |
//! |---|---|
//! | `pAacDecoderChannelInfo[ch]`, `pAacDecoderStaticChannelInfo[ch]` | `Option<Box<...>>` per channel; a new zeroed allocation replaces the old one wherever C allocates (`CAacDecoder_Init()` with a changed configuration) |
//! | `pAacDecoderChannelInfo[ch]->pComData` | [`AacDecoderInstance::com_data`]`[`[`AacDecoderInstance::p_com_data`]`[ch]]`: one [`CAacDecoderCommonData`] per channel element of the configuration, shared by the channels of a CPE (index 0 is `WorkBufferCore6`); a CPE of the bitstream decoded into the channels of two SCEs of the configuration uses both channels' allocations, as C does ([`crate::channel`]) |
//! | `pAacDecoderChannelInfo[ch]->pComStaticData` | [`AacDecoderInstance::com_static_data`]`[`[`AacDecoderInstance::p_com_static_data`]`[ch]]` |
//! | `pComStaticData->pWorkBufferCore1` (one buffer for all elements) | [`AacDecoderInstance::p_work_buffer_core1`] |
//! | `pAacDecoderChannelInfo[ch]->pDynData` | owned by the channel info (`p_dyn_data`); it is zeroed with every new channel info, as `pComData` is newly allocated in C |
//! | `pAacDecoderChannelInfo[ch]->pSpectralCoefficient` | `workBufferCore2[ch * 1024..]` ([`AacDecoderInstance::work_buffer_core2`]) |
//! | `workBufferCore1`, `workBufferCore2`, `pTimeData2` | owned buffers of the instance, reused as in C (so stale contents are reproduced by construction) |
//! | `pUsacConfig[0]` (a pointer into the transport decoder's `asc[0]`) | the flag [`AacDecoderInstance::p_usac_config`]; the configuration is read from `hInput->asc[0]` when used |
//! | `pDrmBsBuffer` | the `DrmBsBuffer` view of [`AacDecoderInstance::p_work_buffer_core1`] |
//! | `streamInfo.pChannelType`, `pChannelIndices` | they always point to `channelType` and `channelIndices`; the port keeps these arrays inside [`CStreamInfo`] |
//!
//! # Bitstream
//!
//! The bitstream of the element loop is the transport decoder's
//! (`transportDec_GetBitstream(hInput, 0)`). The functions of this module work on a copy of it
//! (`FDK_BITSTREAM` is a value with a pointer to the shared bit buffer) and copy it back into the
//! transport decoder before every transport function that reads its position (CRC regions,
//! remaining access unit bits), and back again afterwards (`AacDecoderInstance::with_tp`).
//!
//! # SBR and MPEG Surround
//!
//! The SBR decoder ([`fdk_aac_rs_sbrdec::sbrdecoder`]) and the MPEG Surround decoder
//! ([`fdk_aac_rs_sacdec::sac_dec_lib`]) do not keep a pointer to the QMF domain of the
//! decoder: the glue passes [`AacDecoderInstance::qmf_domain`] to every function that uses it.
//! Their handles are never `NULL` after a successful `aacDecoder_Open()`; where the port's API
//! takes a reference instead of a nullable pointer, an absent handle (unreachable) gives the
//! error the C function returns for `NULL`.

use fdk_aac_rs_arith::ac_arith_coder::carco_create;
use fdk_aac_rs_drc::drc_dec_lib::{
    fdk_drc_dec_init, fdk_drc_dec_read_uni_drc_gain, HandleDrcDecoder,
};
use fdk_aac_rs_fdk::audio::*;
use fdk_aac_rs_fdk::bitstream::{
    escaped_value, fdk_byte_align, fdk_get_valid_bits, fdk_push_back, fdk_push_bi_directional,
    fdk_push_for, fdk_read_bit, fdk_read_bits, FdkBitStream,
};
use fdk_aac_rs_fdk::common_fix::{f_mult, fl2fxconst_sgl, FixpDbl, FixpSgl, FRACT_BITS};
use fdk_aac_rs_fdk::fdk_assert;
use fdk_aac_rs_fdk::mdct::mdct_init;
use fdk_aac_rs_fdk::qmf_domain::{fdk_qmf_domain_clear_requested, FdkQmfDomain};
use fdk_aac_rs_fdk::syslib_channel_map_descr::{
    fdk_ch_map_descr_get_map_value, FdkChannelMapDescr,
};
use fdk_aac_rs_fdk::tools_rom::AC_EL_GA_CCE;
use fdk_aac_rs_pcmutils::limiter::{
    pcm_limiter_set_attack, pcm_limiter_set_threshold, TDLimiterPtr,
};
use fdk_aac_rs_pcmutils::pcmdmx_lib::{
    pcm_dmx_get_param, pcm_dmx_parse, pcm_dmx_reset, pcm_dmx_set_matrix_mixdown_from_pce,
    HandlePcmDownmix, MIN_NUMBER_OF_OUTPUT_CHANNELS, PCMDMX_OK, PCMDMX_RESET_BS_DATA,
};
use fdk_aac_rs_tpdec::tpdec_lib::{
    cprogram_config_compare, cprogram_config_get_default, cprogram_config_get_element_table,
    cprogram_config_get_pce_ch_map, cprogram_config_init, cprogram_config_is_valid,
    cprogram_config_lookup_element, cprogram_config_read, cprogram_config_reset,
    transport_dec_crc_check, transport_dec_crc_end_reg, transport_dec_crc_start_reg,
    transport_dec_get_au_bits_remaining, transport_dec_get_au_bits_total, transport_dec_get_format,
    transport_dec_in_band_config_cb, CProgramConfig, CSAudioSpecificConfig, CSUsacConfig,
    TransportDec, TP_USAC_MAX_CONFIG_LEN, TP_USAC_MAX_ELEMENTS, TRANSPORTDEC_OK,
};

pub use crate::aac_ram::TIME_DATA_FLUSH_SIZE;
use crate::aac_ram::{
    get_aac_decoder_channel_info, get_aac_decoder_static_channel_info, get_cpe_persistent_data,
    get_cplx_prediction_data, get_drc_info, get_overlap_buffer, get_required_mem_work_buffer_core5,
    get_time_data_flush, get_work_buffer_core1, get_work_buffer_core2, get_work_buffer_core5,
    get_work_buffer_core6,
};
use crate::aac_rom::{PcmDec, AACDEC_CH_ELEMENTS_TAB_SIZE, AACDEC_MAX_CH_CONF, ELEMENTS_TAB};
use crate::aacdec_drc::{
    aac_decoder_drc_apply, aac_decoder_drc_disable, aac_decoder_drc_epilog,
    aac_decoder_drc_get_info, aac_decoder_drc_init, aac_decoder_drc_init_channel_data,
    aac_decoder_drc_mark_payload, aac_decoder_drc_prolog, aac_decoder_drc_set_param,
    AACDEC_DRC_GAIN_SCALING, DRC_BS_DELAY, DVB_DRC_ANC_DATA, MPEG_DRC_EXT_DATA,
};
use crate::aacdec_drc_types::CDrcInfo;
use crate::aacdec_pns::{cpns_reset_data, cpns_update_noise_state};
use crate::aacdecoder_lib::{
    CStreamInfo, AACDEC_CLRHIST, AACDEC_CONCEAL, AACDEC_FLUSH, AACDEC_INTR,
};
use crate::block::{cblock_frequency_to_time, cblock_frequency_to_time_low_delay};
use crate::channel::{c_channel_element_decode, c_channel_element_read};
use crate::channelinfo::*;
use crate::conceal::{
    cconcealment_apply, cconcealment_get_delay, cconcealment_get_last_frame_ok,
    cconcealment_init_channel_data, cconcealment_init_common_data, cconcealment_td_fading,
};
use crate::conceal_types::{
    CConcealParams, CConcealmentMethod, ConcealMethodNone, ConcealState_Mute,
};
use crate::delay::{fdk_delay_create, fdk_delay_destroy, FdkSignalDelay};
use crate::overlapadd::OverlapBufferSize;
use crate::usacdec_const::L_DIV;
use crate::usacdec_lpd::clpd_render_time_signal;

use fdk_aac_rs_sacdec::sac_dec_errorcodes::{MPS_INVALID_PARAMETER, MPS_OK};
use fdk_aac_rs_sacdec::sac_dec_lib::{
    mpeg_surround_decoder_configure_qmf_domain, mpeg_surround_decoder_get_lib_info,
    mpeg_surround_decoder_is_full_mpeg_surround_decoder_instance_available,
    mpeg_surround_decoder_is_pseudo_lr, mpeg_surround_decoder_open, mpeg_surround_decoder_parse,
    mpeg_surround_decoder_parse_no_header, mpeg_surround_decoder_set_param, CMpegSurroundDecoder,
    SACDEC_PARTIALLY_COMPLEX, SAC_INSTANCE_NOT_FULL_AVAILABLE, SAC_INTERFACE_QMF,
    SAC_INTERFACE_TIME,
};
use fdk_aac_rs_sbrdec::sbrdecoder::{
    sbr_decoder_init_element, sbr_decoder_parse, sbr_decoder_set_param, SbrDecoderInstance,
    SbrError, SBRDEC_NOT_INITIALIZED, SBRDEC_OK, SBRDEC_PARSE_ERROR, SBR_QMF_MODE,
};

pub mod tables;
use tables::MAP_SBR_RATIO_2_N_ANA_BANDS;

/// `HANDLE_SBRDECODER` (`None` for `NULL`)
pub type HandleSbrDecoder = Option<Box<SbrDecoderInstance>>;

/// `TIME_DATA_FLUSH_SIZE_SF`
pub const TIME_DATA_FLUSH_SIZE_SF: i32 = 7;

/// `AACDEC_MAX_NUM_PREROLL_AU_USAC`
pub const AACDEC_MAX_NUM_PREROLL_AU_USAC: u32 = 3;
/// `AACDEC_MAX_NUM_PREROLL_AU` (undefined before `aacdecoder.h`, so it becomes
/// `AACDEC_MAX_NUM_PREROLL_AU_USAC`)
pub const AACDEC_MAX_NUM_PREROLL_AU: usize = AACDEC_MAX_NUM_PREROLL_AU_USAC as usize;

/// The number of elements of the element tables, the literal
/// `(3 * ((8) * 2) + (((8) * 2)) / 2 + 4 * (1) + 1)` of the C code
/// (`3 * MAX_CHANNELS + MAX_CHANNELS / 2 + 4 * MAX_TRACKS + 1` with 16 channels and 1 track).
pub const MAX_ELEMENTS: usize = 3 * (8 * 2) + (8 * 2) / 2 + 4 + 1;

/// The number of channels of the per channel arrays (`(8)`).
pub(crate) const NCH: usize = 8;

/// `L`
pub const L: usize = 0;
/// `R`
pub const R: usize = 1;

/// The buffer of `CAncData` (`buffer`): where the decoder writes the ancillary data.
#[derive(Debug, Default)]
pub enum AncBuffer {
    /// `NULL`
    #[default]
    None,
    /// A buffer owned by the decoder (given to `aacDecoder_AncDataInit()` by value).
    Owned(Box<[u8]>),
    /// A buffer of the caller (the C ABI).
    Raw(RawAncBuffer),
}

/// A caller's buffer of `len` bytes for the ancillary data (`aacDecoder_AncDataInit()` of the C
/// ABI). It can only be created with [`RawAncBuffer::new`], whose caller guarantees the memory.
#[derive(Debug)]
pub struct RawAncBuffer {
    ptr: core::ptr::NonNull<u8>,
    len: usize,
}

// SAFETY: the memory is only accessed through this value, by the thread that uses the decoder
// that holds it (the contract of `RawAncBuffer::new`).
unsafe impl Send for RawAncBuffer {}

impl RawAncBuffer {
    /// The buffer at `ptr` of `len` bytes (`None` for a null pointer).
    ///
    /// # Safety
    /// `ptr` must be valid for reads and writes of `len` bytes for as long as the returned value
    /// exists, and the memory must not be accessed otherwise during that time.
    pub unsafe fn new(ptr: *mut u8, len: usize) -> Option<RawAncBuffer> {
        core::ptr::NonNull::new(ptr).map(|ptr| RawAncBuffer { ptr, len })
    }

    /// The start of the buffer.
    pub fn as_ptr(&self) -> *mut u8 {
        self.ptr.as_ptr()
    }

    /// The bytes of the buffer.
    pub fn as_slice(&self) -> &[u8] {
        // SAFETY: valid for `len` bytes and not accessed otherwise (contract of `new`).
        unsafe { core::slice::from_raw_parts(self.ptr.as_ptr(), self.len) }
    }

    /// The bytes of the buffer, writable.
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        // SAFETY: see `as_slice`.
        unsafe { core::slice::from_raw_parts_mut(self.ptr.as_ptr(), self.len) }
    }
}

impl AncBuffer {
    fn is_null(&self) -> bool {
        matches!(self, AncBuffer::None)
    }

    /// `buffer[i] = v` (dropped outside the buffer, where the reference has no defined result).
    fn write(&mut self, i: usize, v: u8) {
        let b = match self {
            AncBuffer::None => return,
            AncBuffer::Owned(b) => &mut b[..],
            AncBuffer::Raw(r) => r.as_mut_slice(),
        };
        if let Some(x) = b.get_mut(i) {
            *x = v;
        }
    }
}

/// `CAncData`
#[derive(Debug, Default)]
pub struct CAncData {
    /// `buffer`
    pub buffer: AncBuffer,
    /// `bufferSize`
    pub buffer_size: i32,
    /// `offset`
    pub offset: [i32; 8],
    /// `nrElements`
    pub nr_elements: i32,
}

/// `QMF_MODE`
pub type QmfMode = i32;
/// `NOT_DEFINED`
pub const NOT_DEFINED: QmfMode = -1;
/// `MODE_HQ`
pub const MODE_HQ: QmfMode = 0;
/// `MODE_LP`
pub const MODE_LP: QmfMode = 1;

/// `SBR_PARAMS`
#[derive(Clone, Copy, Debug, Default)]
pub struct SbrParams {
    /// `bsDelay`
    pub bs_delay: i32,
}

/// `AACDEC_FLUSH_OFF`
pub const AACDEC_FLUSH_OFF: u8 = 0;
/// `AACDEC_RSV60_CFG_CHANGE_ATSC_FLUSH_ON`
pub const AACDEC_RSV60_CFG_CHANGE_ATSC_FLUSH_ON: u8 = 1;
/// `AACDEC_RSV60_DASH_IPF_ATSC_FLUSH_ON`
pub const AACDEC_RSV60_DASH_IPF_ATSC_FLUSH_ON: u8 = 2;
/// `AACDEC_USAC_DASH_IPF_FLUSH_ON`
pub const AACDEC_USAC_DASH_IPF_FLUSH_ON: u8 = 3;

/// `AACDEC_BUILD_UP_OFF`
pub const AACDEC_BUILD_UP_OFF: u8 = 0;
/// `AACDEC_RSV60_BUILD_UP_ON`
pub const AACDEC_RSV60_BUILD_UP_ON: u8 = 1;
/// `AACDEC_RSV60_BUILD_UP_ON_IN_BAND`
pub const AACDEC_RSV60_BUILD_UP_ON_IN_BAND: u8 = 2;
/// `AACDEC_USAC_BUILD_UP_ON`
pub const AACDEC_USAC_BUILD_UP_ON: u8 = 3;
/// `AACDEC_RSV60_BUILD_UP_IDLE`
pub const AACDEC_RSV60_BUILD_UP_IDLE: u8 = 4;
/// `AACDEC_RSV60_BUILD_UP_IDLE_IN_BAND`
pub const AACDEC_RSV60_BUILD_UP_IDLE_IN_BAND: u8 = 5;

/// `AACDEC_CROSSFADE_BITMASK_OFF`: no cross-fade between frames shall be applied at the next
/// config change.
pub const AACDEC_CROSSFADE_BITMASK_OFF: u8 = 0;
/// `AACDEC_CROSSFADE_BITMASK_PREROLL`: applyCrossfade is signaled in AudioPreRoll
pub const AACDEC_CROSSFADE_BITMASK_PREROLL: u8 = 1 << 1;

/// `CUsacCoreExtensions`: USAC extension elements
#[derive(Clone, Copy, Debug, Default)]
pub struct CUsacCoreExtensions {
    /// `usacExtElementType`
    pub usac_ext_element_type: [UsacExtElementType; 3],
    /// `usacExtElementDefaultLength`
    pub usac_ext_element_default_length: [u32; 3],
    /// `usacExtElementPayloadFrag`
    pub usac_ext_element_payload_frag: [u8; 3],
}

/// `struct AAC_DECODER_INSTANCE` (`HANDLE_AACDECODER`). See the [module documentation](self)
/// for the members that are pointers in C.
pub struct AacDecoderInstance {
    /// `aacChannels`: amount of AAC decoder channels allocated.
    pub aac_channels: i32,
    /// `ascChannels`: amount of AAC decoder channels signalled in ASC.
    pub asc_channels: [i32; 1],
    /// `blockNumber`: frame counter
    pub block_number: i32,

    /// `nrOfLayers`
    pub nr_of_layers: i32,

    /// `outputInterleaved`: PCM output format (interleaved/none interleaved).
    pub output_interleaved: i32,

    /// `aacOutDataHeadroom`: headroom of the output time signal to prevent clipping
    pub aac_out_data_headroom: i32,

    /// `hInput`: transport layer handle.
    pub h_input: Option<Box<TransportDec>>,

    /// `samplingRateInfo`: sampling rate information table
    pub sampling_rate_info: [SamplingRateInfo; 1],

    /// `frameOK`: will be unset if a consistency check, e.g. CRC etc. fails
    pub frame_ok: u8,

    /// `flags`: flags for internal decoder use. DO NOT USE self::streaminfo::flags !
    pub flags: [u32; 1],
    /// `elFlags`: flags for internal decoder use (element specific).
    pub el_flags: [u32; MAX_ELEMENTS],

    /// `elements`: table where the element Id's are listed
    pub elements: [Mp4ElementId; MAX_ELEMENTS],
    /// `elTags`: table where the elements id Tags are listed
    pub el_tags: [u8; MAX_ELEMENTS],
    /// `chMapping`: table of MPEG canonical order to bitstream channel order mapping.
    pub ch_mapping: [u8; 8 * 2],

    /// `mapDescr`: describes the output channel mapping.
    pub map_descr: FdkChannelMapDescr,
    /// `chMapIndex`: index to access one line of the channelOutputMapping table.
    pub ch_map_index: u8,
    /// `sbrDataLen`: expected length of the SBR remaining in bitbuffer after the AAC payload
    /// has been pared.
    pub sbr_data_len: i32,

    /// `pce`
    pub pce: CProgramConfig,
    /// `streamInfo` (also holds `channelType` and `channelIndices`, see the module
    /// documentation)
    pub stream_info: CStreamInfo,
    /// `pAacDecoderChannelInfo`: temporal channel memory
    pub p_aac_decoder_channel_info: [Option<Box<CAacDecoderChannelInfo>>; NCH],
    /// `pAacDecoderStaticChannelInfo`: persistent channel memory
    pub p_aac_decoder_static_channel_info: [Option<Box<CAacDecoderStaticChannelInfo>>; NCH],

    /// `pAacDecoderChannelInfo[ch]->pComData`: index into `com_data` (`None` for `NULL`).
    pub p_com_data: [Option<usize>; NCH],
    /// `pAacDecoderChannelInfo[ch]->pComStaticData`: index into `com_static_data`.
    pub p_com_static_data: [Option<usize>; NCH],
    /// The `CAacDecoderCommonData` allocations (index 0: `WorkBufferCore6`).
    pub com_data: Vec<Box<CAacDecoderCommonData>>,
    /// The `CAacDecoderCommonStaticData` allocations.
    pub com_static_data: Vec<Box<CAacDecoderCommonStaticData>>,
    /// `pComStaticData->pWorkBufferCore1`: shared by all elements.
    pub p_work_buffer_core1: Option<Box<CWorkBufferCore1>>,

    /// `workBufferCore1`
    pub work_buffer_core1: Option<Box<CWorkBufferCore1>>,
    /// `workBufferCore2`
    pub work_buffer_core2: Option<Vec<FixpDbl>>,
    /// `pTimeData2`
    pub p_time_data2: Option<Vec<PcmDec>>,
    /// `timeData2Size` (bytes)
    pub time_data2_size: i32,

    /// `cpeStaticData`: persistent data shared by both channels of a CPE.
    pub cpe_static_data: [Option<Box<CpePersistentData>>; MAX_ELEMENTS],

    /// `concealCommonData`
    pub conceal_common_data: CConcealParams,
    /// `concealMethodUser`
    pub conceal_method_user: CConcealmentMethod,

    /// `usacCoreExt`: data and handles to extend USAC FD/LPD core decoder (SBR, MPS, ...)
    pub usac_core_ext: CUsacCoreExtensions,
    /// `numUsacElements`
    pub num_usac_elements: [u32; 1],
    /// `usacStereoConfigIndex`
    pub usac_stereo_config_index: [u8; MAX_ELEMENTS],
    /// `pUsacConfig[0] != NULL` (it always points to `hInput->asc[0].m_sc.m_usacConfig`).
    pub p_usac_config: [bool; 1],
    /// `nbDiv`: number of frame divisions in LPD-domain
    pub nb_div: i32,

    /// `useLdQmfTimeAlign`
    pub use_ld_qmf_time_align: u8,

    /// `aacChannelsPrev`: the amount of AAC core channels of the last successful decode call.
    pub aac_channels_prev: i32,
    /// `channelTypePrev`: the channelType values of the last successful decode call.
    pub channel_type_prev: [AudioChannelType; 8],
    /// `channelIndicesPrev`: the channelIndices values of the last successful decode call.
    pub channel_indices_prev: [u8; 8],

    /// `downscaleFactor`: a supported ELD downscale factor of 1, 2, 3 or 4
    pub downscale_factor: u8,
    /// `downscaleFactorInBS`: the (not necessarily supported) ELD downscale factor discovered
    /// in the bitstream
    pub downscale_factor_in_bs: u8,

    /// `hSbrDecoder`: SBR decoder handle
    pub h_sbr_decoder: HandleSbrDecoder,
    /// `sbrEnabled`: flag to store if SBR has been detected
    pub sbr_enabled: u8,
    /// `sbrEnabledPrev`: flag to store if SBR has been detected from previous frame
    pub sbr_enabled_prev: u8,
    /// `psPossible`: flag to store if PS is possible
    pub ps_possible: u8,
    /// `sbrParams`: struct to store all sbr parameters
    pub sbr_params: SbrParams,

    /// `pDrmBsBuffer != NULL` (it points to the `DrmBsBuffer` of `p_work_buffer_core1`).
    pub p_drm_bs_buffer: bool,
    /// `drmBsBufferSize`
    pub drm_bs_buffer_size: u16,
    /// `qmfDomain`: instance of module for QMF domain data handling
    pub qmf_domain: FdkQmfDomain,

    /// `qmfModeCurr`: the current QMF mode
    pub qmf_mode_curr: QmfMode,
    /// `qmfModeUser`: the QMF mode requested by the library user
    pub qmf_mode_user: QmfMode,

    /// `hDrcInfo`: handle to DRC data structure
    pub h_drc_info: Option<Box<CDrcInfo>>,
    /// `metadataExpiry`: metadata expiry time in milli-seconds.
    pub metadata_expiry: i32,

    /// `pMpegSurroundDecoder`: MPEG Surround decoder handle
    pub p_mpeg_surround_decoder: Option<Box<CMpegSurroundDecoder>>,
    /// `mpsEnableUser`: MPS enable user flag
    pub mps_enable_user: u8,
    /// `mpsEnableCurr`: MPS enable decoder state
    pub mps_enable_curr: u8,
    /// `mpsApplicable`: MPS applicable
    pub mps_applicable: u8,
    /// `mpsOutputMode`: setting: normal = 0, binaural = 1, stereo = 2, 5.1ch = 3
    pub mps_output_mode: i8,
    /// `mpsOutChannelsLast`: the amount of channels returned by the last successful MPS
    /// decoder call.
    pub mps_out_channels_last: i32,
    /// `mpsFrameSizeLast`: the frame length returned by the last successful MPS decoder call.
    pub mps_frame_size_last: i32,

    /// `ancData`: structure to handle ancillary data
    pub anc_data: CAncData,

    /// `hPcmUtils`: privat data for the PCM utils.
    pub h_pcm_utils: Option<HandlePcmDownmix>,

    /// `hLimiter`: handle of time domain limiter.
    pub h_limiter: Option<TDLimiterPtr>,
    /// `limiterEnableUser`: the limiter configuration requested by the library user
    pub limiter_enable_user: u8,
    /// `limiterEnableCurr`: the current limiter configuration.
    pub limiter_enable_curr: u8,

    /// `extGain`: gain that must be applied to the output signal.
    pub ext_gain: [FixpDbl; 1],
    /// `extGainDelay`: delay that must be accounted for extGain.
    pub ext_gain_delay: u32,

    /// `hUniDrcDecoder`
    pub h_uni_drc_decoder: HandleDrcDecoder,
    /// `multibandDrcPresent`
    pub multiband_drc_present: u8,
    /// `numTimeSlots`
    pub num_time_slots: u8,
    /// `loudnessInfoSetPosition`
    pub loudness_info_set_position: [u32; 3],
    /// `defaultTargetLoudness`
    pub default_target_loudness: i8,

    /// `pTimeDataFlush`: the flushed time data which will be used for the crossfade in case of
    /// an USAC DASH IPF config change
    pub p_time_data_flush: [Option<Vec<PcmDec>>; 8 * 2],

    /// `flushStatus`: indicates flush status: on|off
    pub flush_status: u8,
    /// `flushCnt`: flush frame counter
    pub flush_cnt: i8,
    /// `buildUpStatus`: indicates build up status: on|off
    pub build_up_status: u8,
    /// `buildUpCnt`: build up frame counter
    pub build_up_cnt: i8,
    /// `hasAudioPreRoll`: indicates preRoll status: on|off
    pub has_audio_pre_roll: u8,
    /// `prerollAULength`: relative offset of the prerollAU end position to the AU start
    /// position in the bitstream
    pub preroll_au_length: [u32; AACDEC_MAX_NUM_PREROLL_AU + 1],
    /// `accessUnit`: number of the actual processed preroll accessUnit
    pub access_unit: i32,
    /// `applyCrossfade`: if any bit is set, cross-fade for seamless stream switching is applied
    pub apply_crossfade: u8,

    /// `usacResidualDelay`: delay residual signal to compensate for eSBR delay of DMX signal
    /// in case of stereoConfigIndex==2.
    pub usac_residual_delay: FdkSignalDelay,

    /// Not in C: the frame pipeline of a decoder with more than one thread
    /// ([`crate::pipeline`]); `None` for the sequential decoder (the default).
    pub pipe: Option<Box<crate::pipeline::Pipeline>>,
    /// Not in C: the thread pool and scratch memory of the per channel parallelism of a decoder
    /// with more than one thread ([`crate::pipeline::chan`]; owned by stage B of the pipeline).
    pub chan_par: crate::pipeline::chan::ChanPar,
    /// Not in C: incremented whenever the channel memory is (re)allocated or freed
    /// (`CAacDecoder_Init()`, `CAacDecoder_DeInit()`); the frame pipeline uses it to know when the
    /// second set of per frame channel memory has to be rebuilt.
    pub d_gen: u32,
}

/// `HANDLE_AACDECODER`
pub type HandleAacDecoder<'a> = &'a mut AacDecoderInstance;

impl AacDecoderInstance {
    /// `GetAacDecoder()`: a zeroed instance (`FDKcalloc`).
    pub(crate) fn zeroed() -> Box<Self> {
        Box::new(AacDecoderInstance {
            aac_channels: 0,
            asc_channels: [0; 1],
            block_number: 0,
            nr_of_layers: 0,
            output_interleaved: 0,
            aac_out_data_headroom: 0,
            h_input: None,
            sampling_rate_info: [SamplingRateInfo::default(); 1],
            frame_ok: 0,
            flags: [0; 1],
            el_flags: [0; MAX_ELEMENTS],
            elements: [0; MAX_ELEMENTS],
            el_tags: [0; MAX_ELEMENTS],
            ch_mapping: [0; 8 * 2],
            map_descr: FdkChannelMapDescr::default(),
            ch_map_index: 0,
            sbr_data_len: 0,
            pce: CProgramConfig::zeroed(),
            stream_info: CStreamInfo::default(),
            p_aac_decoder_channel_info: Default::default(),
            p_aac_decoder_static_channel_info: Default::default(),
            p_com_data: [None; NCH],
            p_com_static_data: [None; NCH],
            com_data: Vec::new(),
            com_static_data: Vec::new(),
            p_work_buffer_core1: None,
            work_buffer_core1: None,
            work_buffer_core2: None,
            p_time_data2: None,
            time_data2_size: 0,
            cpe_static_data: core::array::from_fn(|_| None),
            conceal_common_data: CConcealParams::default(),
            conceal_method_user: 0,
            usac_core_ext: CUsacCoreExtensions::default(),
            num_usac_elements: [0; 1],
            usac_stereo_config_index: [0; MAX_ELEMENTS],
            p_usac_config: [false; 1],
            nb_div: 0,
            use_ld_qmf_time_align: 0,
            aac_channels_prev: 0,
            channel_type_prev: [0; 8],
            channel_indices_prev: [0; 8],
            downscale_factor: 0,
            downscale_factor_in_bs: 0,
            h_sbr_decoder: None,
            sbr_enabled: 0,
            sbr_enabled_prev: 0,
            ps_possible: 0,
            sbr_params: SbrParams::default(),
            p_drm_bs_buffer: false,
            drm_bs_buffer_size: 0,
            qmf_domain: FdkQmfDomain::default(),
            qmf_mode_curr: 0,
            qmf_mode_user: 0,
            h_drc_info: None,
            metadata_expiry: 0,
            p_mpeg_surround_decoder: None,
            mps_enable_user: 0,
            mps_enable_curr: 0,
            mps_applicable: 0,
            mps_output_mode: 0,
            mps_out_channels_last: 0,
            mps_frame_size_last: 0,
            anc_data: CAncData::default(),
            h_pcm_utils: None,
            h_limiter: None,
            limiter_enable_user: 0,
            limiter_enable_curr: 0,
            ext_gain: [0; 1],
            ext_gain_delay: 0,
            h_uni_drc_decoder: None,
            multiband_drc_present: 0,
            num_time_slots: 0,
            loudness_info_set_position: [0; 3],
            default_target_loudness: 0,
            p_time_data_flush: Default::default(),
            flush_status: 0,
            flush_cnt: 0,
            build_up_status: 0,
            build_up_cnt: 0,
            has_audio_pre_roll: 0,
            preroll_au_length: [0; AACDEC_MAX_NUM_PREROLL_AU + 1],
            access_unit: 0,
            apply_crossfade: 0,
            usac_residual_delay: FdkSignalDelay::default(),
            pipe: None,
            chan_par: Default::default(),
            d_gen: 0,
        })
    }

    /// Runs `f` on the transport decoder with its bitstream set to `bs`, and copies the
    /// bitstream back into `bs` afterwards (see the [module documentation](self#bitstream)).
    /// Without a transport decoder (only inside a transport callback) `f` is not run.
    pub(crate) fn with_tp<T: Default>(
        &mut self,
        bs: &mut FdkBitStream,
        f: impl FnOnce(&mut TransportDec) -> T,
    ) -> T {
        match self.h_input.as_deref_mut() {
            Some(tp) => {
                tp.bit_stream[0] = bs.clone();
                let r = f(tp);
                *bs = tp.bit_stream[0].clone();
                r
            }
            None => T::default(),
        }
    }

    /// The USAC configuration `pUsacConfig[0]` points to (`hInput->asc[0].m_sc.m_usacConfig`).
    pub(crate) fn usac_config(&self) -> Option<&CSUsacConfig> {
        if self.p_usac_config[0] {
            self.h_input.as_deref().map(|tp| &tp.asc[0].m_sc.m_usac_config)
        } else {
            None
        }
    }
}

/// `PS_IS_EXPLICITLY_DISABLED(aot, flags)`
#[inline]
fn ps_is_explicitly_disabled(aot: AudioObjectType, flags: u32) -> bool {
    (aot == AOT_DRM_AAC) && (flags & AC_PS_PRESENT) == 0
}

/// `IS_STEREO_SBR(el_id, stereoConfigIndex)`
#[inline]
pub(crate) fn is_stereo_sbr(el_id: Mp4ElementId, stereo_config_index: u8) -> bool {
    (el_id == ID_USAC_CPE && stereo_config_index == 0)
        || (el_id == ID_USAC_CPE && stereo_config_index == 3)
}

/// `CAacDecoder_SyncQmfMode(self)`: synchronise the QMF mode for all modules using QMF data.
pub fn caac_decoder_sync_qmf_mode(self_: &mut AacDecoderInstance) {
    fdk_assert!(!((self_.flags[0] & AC_MPS_PRESENT) != 0 && (self_.flags[0] & AC_PS_PRESENT) != 0));

    /* Assign user requested mode */
    self_.qmf_mode_curr = self_.qmf_mode_user;

    if is_usac(self_.stream_info.aot) {
        self_.qmf_mode_curr = MODE_HQ;
    }

    if self_.qmf_mode_curr == NOT_DEFINED {
        if (is_lowdelay(self_.stream_info.aot) && (self_.flags[0] & AC_MPS_PRESENT) != 0)
            || ((self_.stream_info.aac_num_channels == 1)
                && ((can_do_ps(self_.stream_info.aot) && (self_.flags[0] & AC_MPS_PRESENT) == 0)
                    || is_usac(self_.stream_info.aot)))
        {
            self_.qmf_mode_curr = MODE_HQ;
        } else {
            self_.qmf_mode_curr = MODE_LP;
        }
    }

    if self_.mps_enable_curr != 0 {
        if is_lowdelay(self_.stream_info.aot) && (self_.qmf_mode_curr == MODE_LP) {
            /* Overrule user requested QMF mode */
            self_.qmf_mode_curr = MODE_HQ;
        }
        /* Set and check if MPS decoder allows the current mode */
        let r = mpeg_surround_decoder_set_param(
            self_.p_mpeg_surround_decoder.as_deref_mut(),
            SACDEC_PARTIALLY_COMPLEX,
            (self_.qmf_mode_curr == MODE_LP) as i32,
        );
        let mut supported = r == MPS_OK;
        if r == MPS_INVALID_PARAMETER {
            /* Only one mode supported. Find out which one: */
            let mut lib_info = [LibInfo::default(); FDK_MODULE_LAST as usize];

            fdk_init_lib_info(&mut lib_info);
            mpeg_surround_decoder_get_lib_info(Some(&mut lib_info));
            let mps_caps = fdk_lib_info_get_capabilities(&lib_info, FDK_MPSDEC);

            if ((mps_caps & CAPF_MPS_LP) != 0 && (self_.qmf_mode_curr == MODE_LP))
                || ((mps_caps & CAPF_MPS_HQ) != 0 && (self_.qmf_mode_curr == MODE_HQ))
            {
                /* MPS decoder does support the requested mode. */
                supported = true;
            }
        }
        if !supported {
            if self_.qmf_mode_user == NOT_DEFINED {
                /* Revert in case mpegSurroundDecoder_SetParam() fails. */
                self_.qmf_mode_curr =
                    if self_.qmf_mode_curr == MODE_LP { MODE_HQ } else { MODE_LP };
            } else {
                /* in case specific mode was requested we disable MPS and playout the downmix */
                self_.mps_enable_curr = 0;
            }
        }
    }

    /* Set SBR to current QMF mode. Error does not matter. */
    sbr_decoder_set_param(
        self_.h_sbr_decoder.as_deref_mut(),
        SBR_QMF_MODE,
        (self_.qmf_mode_curr == MODE_LP) as i32,
    );
    self_.ps_possible = ((can_do_ps(self_.stream_info.aot)
        && !ps_is_explicitly_disabled(self_.stream_info.aot, self_.flags[0])
        && self_.stream_info.aac_num_channels == 1
        && (self_.flags[0] & AC_MPS_PRESENT) == 0)
        && self_.qmf_mode_curr == MODE_HQ) as u8;
    fdk_assert!(!((self_.flags[0] & AC_MPS_PRESENT) != 0 && self_.ps_possible != 0));
}

/// `CAacDecoder_SignalInterruption(self)`: signal a bit stream interruption to the decoder.
pub fn caac_decoder_signal_interruption(self_: &mut AacDecoderInstance) {
    if self_.flags[0] & (AC_USAC | AC_RSVD50 | AC_RSV603DA) != 0 {
        let n = self_.aac_channels.min(NCH as i32);
        for i in 0..n.max(0) as usize {
            if let Some(sci) = self_.p_aac_decoder_static_channel_info[i].as_deref_mut() {
                /* number of active channels can be smaller */
                // hArCo exists for every USAC channel (CAacDecoder_Init() creates it).
                if let Some(arco) = sci.h_ar_co.as_deref_mut() {
                    arco.m_number_lines_prev = 0;
                }
            }
        }
    }
}

/// `CAacDecoder_GetELChannels(type, usacStereoConfigIndex)` (static in C): the number of
/// channels of an element.
pub fn caac_decoder_get_el_channels(type_: Mp4ElementId, usac_stereo_config_index: u8) -> i32 {
    match type_ {
        ID_USAC_CPE => {
            if usac_stereo_config_index == 1 {
                1
            } else {
                2
            }
        }
        ID_CPE => 2,
        ID_USAC_SCE | ID_USAC_LFE | ID_SCE | ID_LFE => 1,
        _ => 0,
    }
}

/// `CAacDecoder_AncDataReset(ancData)` (static in C): reset the ancillary data struct. Call
/// before parsing a new frame.
pub fn caac_decoder_anc_data_reset(anc_data: &mut CAncData) -> AacDecoderError {
    for i in 0..8 {
        anc_data.offset[i] = 0;
    }
    anc_data.nr_elements = 0;

    AAC_DEC_OK
}

/// `CAacDecoder_AncDataInit(ancData, buffer, size)`: initialize the ancillary buffer.
pub fn caac_decoder_anc_data_init(
    anc_data: &mut CAncData,
    buffer: AncBuffer,
    size: i32,
) -> AacDecoderError {
    if size >= 0 {
        anc_data.buffer = buffer;
        anc_data.buffer_size = size;

        caac_decoder_anc_data_reset(anc_data);

        return AAC_DEC_OK;
    }

    AAC_DEC_ANC_DATA_ERROR
}

/// `CAacDecoder_AncDataGet(ancData, index, ptr, size)`: get one ancillary data element. `ptr`
/// receives the offset of the element in the ancillary data buffer (`None` for `NULL`).
pub fn caac_decoder_anc_data_get(
    anc_data: &CAncData,
    index: i32,
    ptr: &mut Option<usize>,
    size: &mut i32,
) -> AacDecoderError {
    let error = AAC_DEC_OK;

    *ptr = None;
    *size = 0;

    if index >= 0 && index < 8 - 1 && index < anc_data.nr_elements {
        let i = index as usize;
        // &ancData->buffer[offset] is a pointer even for a NULL buffer (the elements are only
        // counted with a buffer).
        *ptr = Some(anc_data.offset[i] as usize);
        *size = anc_data.offset[i + 1] - anc_data.offset[i];
    }

    error
}

/// `CAacDecoder_AncDataParse(ancData, hBs, ancBytes)` (static in C): parse ancillary data.
pub fn caac_decoder_anc_data_parse(
    anc_data: &mut CAncData,
    h_bs: &mut FdkBitStream,
    anc_bytes: i32,
) -> AacDecoderError {
    let mut error = AAC_DEC_OK;
    let mut read_bytes: i32 = 0;

    if !anc_data.buffer.is_null() {
        if anc_bytes > 0 {
            /* write ancillary data to external buffer */
            // nrElements is at most 7 here (it is checked against 8 - 1 below).
            let offset = anc_data.offset[(anc_data.nr_elements as usize).min(7)];

            if offset.wrapping_add(anc_bytes) > anc_data.buffer_size {
                error = AAC_DEC_TOO_SMALL_ANC_BUFFER;
            } else if anc_data.nr_elements >= 8 - 1 {
                error = AAC_DEC_TOO_MANY_ANC_ELEMENTS;
            } else {
                for i in 0..anc_bytes {
                    let v = fdk_read_bits(h_bs, 8) as u8;
                    anc_data.buffer.write((i + offset) as usize, v);
                    read_bytes += 1;
                }

                anc_data.nr_elements += 1;
                let n = anc_data.nr_elements as usize;
                anc_data.offset[n] = anc_bytes + anc_data.offset[n - 1];
            }
        }
    }

    read_bytes = anc_bytes - read_bytes;

    if read_bytes > 0 {
        /* skip data */
        fdk_push_for(h_bs, (read_bytes << 3) as u32);
    }

    error
}

/// `CDataStreamElement_Read(self, bs, elementInstanceTag, alignmentAnchor)` (static in C): read
/// a data stream element.
pub fn cdata_stream_element_read(
    self_: &mut AacDecoderInstance,
    bs: &mut FdkBitStream,
    element_instance_tag: &mut u8,
    alignment_anchor: u32,
) -> AacDecoderError {
    let crc_reg = self_.with_tp(bs, |tp| transport_dec_crc_start_reg(tp, 0));

    /* Element Instance Tag */
    *element_instance_tag = fdk_read_bits(bs, 4) as u8;
    /* Data Byte Align Flag */
    let data_byte_align_flag = fdk_read_bits(bs, 1);

    let mut count = fdk_read_bits(bs, 8) as i32;

    if count == 255 {
        count += fdk_read_bits(bs, 8) as i32; /* EscCount */
    }
    let dse_bits: u32 = (count * 8) as u32;

    if data_byte_align_flag != 0 {
        fdk_byte_align(bs, alignment_anchor);
    }

    let data_start = fdk_get_valid_bits(bs) as i32;

    let mut error = caac_decoder_anc_data_parse(&mut self_.anc_data, bs, count);
    self_.with_tp(bs, |tp| transport_dec_crc_end_reg(tp, crc_reg));

    {
        /* Move to the beginning of the data chunk */
        let v = fdk_get_valid_bits(bs) as i32;
        fdk_push_back(bs, data_start.wrapping_sub(v) as u32);

        /* Read Anc data if available */
        aac_decoder_drc_mark_payload(self_.h_drc_info.as_deref_mut(), bs, DVB_DRC_ANC_DATA);
    }

    {
        /* Move to the beginning of the data chunk */
        let v = fdk_get_valid_bits(bs) as i32;
        fdk_push_back(bs, data_start.wrapping_sub(v) as u32);

        /* Read DMX meta-data */
        let dmx_err = pcm_dmx_parse(
            self_.h_pcm_utils.as_deref_mut(),
            Some(bs),
            dse_bits,
            0, /* not mpeg2 */
        );
        if error == AAC_DEC_OK && dmx_err != PCMDMX_OK {
            error = AAC_DEC_UNKNOWN;
        }
    }

    /* Move to the very end of the element. */
    let v = fdk_get_valid_bits(bs) as i32;
    fdk_push_bi_directional(bs, v.wrapping_sub(data_start).wrapping_add(dse_bits as i32));

    error
}

/// `findElementInstanceTag(elementTag, elementId, pAacDecoderChannelInfo, nChannels,
/// pElementIdTab, nElements)` (static in C)
pub fn find_element_instance_tag(
    element_tag: i32,
    element_id: Mp4ElementId,
    p_aac_decoder_channel_info: &[Option<Box<CAacDecoderChannelInfo>>],
    n_channels: i32,
    p_element_id_tab: &[Mp4ElementId],
    n_elements: i32,
) -> i32 {
    let mut ch_cnt: i32 = 0;

    for el in 0..n_elements.max(0) as usize {
        let id = p_element_id_tab.get(el).copied().unwrap_or(ID_NONE);
        match id {
            ID_CPE | ID_SCE | ID_LFE => {
                // The channel infos below nChannels exist (C dereferences them).
                let tag = p_aac_decoder_channel_info
                    .get(ch_cnt as usize)
                    .and_then(|c| c.as_deref())
                    .map_or(0, |c| c.element_instance_tag as i32);
                if (element_tag == tag) && (element_id == id) {
                    return 1; /* element instance tag found */
                }
                ch_cnt += if id == ID_CPE { 2 } else { 1 };
            }
            _ => {}
        }
        if ch_cnt >= n_channels {
            break;
        }
        if id == ID_END {
            break;
        }
    }

    0 /* element instance tag not found */
}

/// `validateElementInstanceTags(pce, pAacDecoderChannelInfo, nChannels, pElementIdTab,
/// nElements)` (static in C)
pub fn validate_element_instance_tags(
    pce: &CProgramConfig,
    p_aac_decoder_channel_info: &[Option<Box<CAacDecoderChannelInfo>>],
    n_channels: i32,
    p_element_id_tab: &[Mp4ElementId],
    n_elements: i32,
) -> i32 {
    if n_channels >= pce.num_channels as i32 {
        let f = |tag: u8, id: Mp4ElementId| {
            find_element_instance_tag(
                tag as i32,
                id,
                p_aac_decoder_channel_info,
                n_channels,
                p_element_id_tab,
                n_elements,
            )
        };
        for el in 0..pce.num_front_channel_elements as usize {
            let id = if pce.front_element_is_cpe[el] != 0 { ID_CPE } else { ID_SCE };
            if f(pce.front_element_tag_select[el], id) == 0 {
                return 0; /* element instance tag not in raw_data_block() */
            }
        }
        for el in 0..pce.num_side_channel_elements as usize {
            let id = if pce.side_element_is_cpe[el] != 0 { ID_CPE } else { ID_SCE };
            if f(pce.side_element_tag_select[el], id) == 0 {
                return 0; /* element instance tag not in raw_data_block() */
            }
        }
        for el in 0..pce.num_back_channel_elements as usize {
            let id = if pce.back_element_is_cpe[el] != 0 { ID_CPE } else { ID_SCE };
            if f(pce.back_element_tag_select[el], id) == 0 {
                return 0; /* element instance tag not in raw_data_block() */
            }
        }
        for el in 0..pce.num_lfe_channel_elements as usize {
            if f(pce.lfe_element_tag_select[el], ID_LFE) == 0 {
                return 0; /* element instance tag not in raw_data_block() */
            }
        }
    } else {
        return 0; /* too less decoded audio channels */
    }

    1 /* all element instance tags found in raw_data_block() */
}

/// `CProgramConfigElement_Read(bs, pTp, pce, channelConfig, alignAnchor)` (static in C): read a
/// program config element. Returns the PCE status (-1: fail, 0: no new PCE, 1: PCE updated, 2:
/// PCE updated need re-config).
///
/// `bs` is the transport decoder's bitstream (`pTp`), see `AacDecoderInstance::with_tp`.
pub fn cprogram_config_element_read(
    bs: &mut FdkBitStream,
    p_tp: Option<&mut TransportDec>,
    pce: &mut CProgramConfig,
    channel_config: u32,
    align_anchor: u32,
) -> i32 {
    let mut pce_status = 0;

    /* read PCE to temporal buffer first */
    let mut tmp_pce = CProgramConfig::zeroed();

    cprogram_config_init(&mut tmp_pce);

    let mut p_tp = p_tp;
    let crc_reg = match p_tp.as_deref_mut() {
        Some(tp) => {
            tp.bit_stream[0] = bs.clone();
            let r = transport_dec_crc_start_reg(tp, 0);
            *bs = tp.bit_stream[0].clone();
            r
        }
        None => 0,
    };

    cprogram_config_read(&mut tmp_pce, bs, align_anchor);

    if let Some(tp) = p_tp {
        tp.bit_stream[0] = bs.clone();
        transport_dec_crc_end_reg(tp, crc_reg);
        *bs = tp.bit_stream[0].clone();
    }

    if cprogram_config_is_valid(&tmp_pce) != 0 && (tmp_pce.profile == 1) {
        if cprogram_config_is_valid(pce) == 0 && (channel_config > 0) {
            /* Create a standard channel config PCE to compare with */
            cprogram_config_get_default(pce, channel_config);
        }

        if cprogram_config_is_valid(pce) != 0 {
            /* Compare the new and the old PCE (tags ignored) */
            match cprogram_config_compare(pce, &tmp_pce) {
                1 => {
                    /* Channel configuration not changed. Just new metadata. */
                    *pce = tmp_pce; /* Store the complete PCE */
                    pce_status = 1; /* New PCE but no change of config */
                }
                2 | -1 => {
                    /* The number of channels are identical but not the config, or the channel
                     * configuration is completely different */
                    pce_status = -1; /* Not supported! */
                }
                _ => {
                    /* 0: Nothing to do because PCE matches the old one exactly. */
                    /* pceStatus = 0; */
                }
            }
        }
    }

    pce_status
}

/// `CAacDecoder_PrepareCrossFade(pTimeData, pTimeDataFlush, numChannels, frameSize,
/// interleaved)`: prepares the crossfade for an USAC DASH IPF config change.
pub fn caac_decoder_prepare_cross_fade(
    p_time_data: &[PcmDec],
    p_time_data_flush: &mut [Option<Vec<PcmDec>>],
    num_channels: i32,
    frame_size: i32,
    interleaved: i32,
) -> AacDecoderError {
    let (s1, s2) = if interleaved != 0 { (1, num_channels) } else { (frame_size, 1) };

    for ch in 0..num_channels.max(0) {
        let mut p_in = (ch * s1) as usize;
        // pTimeDataFlush[ch] is allocated for the flushed channels (C dereferences it).
        let flush = p_time_data_flush.get_mut(ch as usize).and_then(|f| f.as_deref_mut());
        let mut tmp = [0 as PcmDec; TIME_DATA_FLUSH_SIZE as usize];
        let dst: &mut [PcmDec] = match flush {
            Some(f) => f,
            None => &mut tmp,
        };
        for i in 0..TIME_DATA_FLUSH_SIZE as usize {
            if let Some(d) = dst.get_mut(i) {
                *d = p_time_data.get(p_in).copied().unwrap_or(0);
            }
            p_in = p_in.wrapping_add(s2 as usize);
        }
    }

    AAC_DEC_OK
}

/// `CAacDecoder_ApplyCrossFade(pTimeData, pTimeDataFlush, numChannels, frameSize,
/// interleaved)`: applies the crossfade for an USAC DASH IPF config change.
pub fn caac_decoder_apply_cross_fade(
    p_time_data: &mut [PcmDec],
    p_time_data_flush: &[Option<Vec<PcmDec>>],
    num_channels: i32,
    frame_size: i32,
    interleaved: i32,
) -> AacDecoderError {
    let (s1, s2) = if interleaved != 0 { (1, num_channels) } else { (frame_size, 1) };

    for ch in 0..num_channels.max(0) {
        let mut p_in = (ch * s1) as usize;
        let flush = p_time_data_flush.get(ch as usize).and_then(|f| f.as_deref());
        for i in 0..TIME_DATA_FLUSH_SIZE {
            let alpha: FixpSgl =
                ((i as FixpSgl) << (FRACT_BITS - 1 - TIME_DATA_FLUSH_SIZE_SF)) as FixpSgl;
            let time: FixpDbl = p_time_data.get(p_in).copied().unwrap_or(0);
            let time_flush: FixpDbl = flush.and_then(|f| f.get(i as usize)).copied().unwrap_or(0);

            if let Some(x) = p_time_data.get_mut(p_in) {
                *x = time_flush
                    .wrapping_sub(f_mult(time_flush, alpha))
                    .wrapping_add(f_mult(time, alpha));
            }
            p_in = p_in.wrapping_add(s2 as usize);
        }
    }

    AAC_DEC_OK
}

/// `CAacDecoder_PreRollExtensionPayloadParse(self, numPrerollAU, prerollAUOffset,
/// prerollAULength)`: parse the preRoll extension payload. The transport decoder's bitstream
/// is restored at the end, as in C.
pub fn caac_decoder_pre_roll_extension_payload_parse(
    self_: &mut AacDecoderInstance,
    num_preroll_au: &mut u32,
    preroll_au_offset: &mut [u32],
    preroll_au_length: &mut [u32],
) -> AacDecoderError {
    let mut error_status = AAC_DEC_OK;

    let mut config_length: u32 = 0;
    let mut preroll_possible: u32 = 1;
    let mut config_changed: u8 = 0;
    let mut config = [0u8; TP_USAC_MAX_CONFIG_LEN];
    /* in case implicit and explicit config is equal preroll AU's should be processed after
     * decoder reset */
    let mut implicit_explicit_cfg_diff: u8 = 0;

    let Some(tp) = self_.h_input.as_deref_mut() else {
        return AAC_DEC_UNKNOWN;
    };
    let bs: FdkBitStream = tp.bit_stream[0].clone();
    let mut h_bs: FdkBitStream = bs.clone();

    'bail: {
        let au_start_anchor = fdk_get_valid_bits(&mut h_bs) as i32;
        if au_start_anchor <= 0 {
            error_status = AAC_DEC_NOT_ENOUGH_BITS;
            break 'bail;
        }

        /* Independency flag */
        fdk_read_bit(&mut h_bs);

        /* Payload present flag of extension ID_EXT_ELE_AUDIOPREROLL must be one */
        let ext_payload_present_flag = fdk_read_bits(&mut h_bs, 1);
        if ext_payload_present_flag == 0 {
            preroll_possible = 0;
        }

        /* Default length flag of extension ID_EXT_ELE_AUDIOPREROLL must be zero */
        let use_default_length_flag = fdk_read_bits(&mut h_bs, 1);
        if use_default_length_flag != 0 {
            preroll_possible = 0;
        }

        if preroll_possible != 0 {
            /* extPayloadPresentFlag && !useDefaultLengthFlag */
            /* Read overall ext payload length, useDefaultLengthFlag must be zero.  */
            escaped_value(&mut h_bs, 8, 16, 0);

            /* Read RSVD60 Config size */
            config_length = escaped_value(&mut h_bs, 4, 4, 8);

            /* Avoid decoding pre roll frames if there was no config change and no config is
             * included in the pre roll ext payload. */
        }

        /* If pre roll not possible then exit. */
        if preroll_possible == 0 {
            /* Sanity check: if flushing is switched on, preRollPossible must be 1 */
            if self_.flush_status != AACDEC_FLUSH_OFF {
                /* Mismatch of current payload and flushing status */
                self_.flush_status = AACDEC_FLUSH_OFF;
                error_status = AAC_DEC_PARSE_ERROR;
            }
            break 'bail;
        }

        if self_.flags[0] & AC_USAC != 0 {
            if config_length > 0 {
                /* DASH IPF USAC Config Change: Read new config and compare with current config.
                 * Apply reconfiguration if config's are different. */
                for i in 0..config_length as usize {
                    // configLength is at most 15 + 15 + 255 = 285 (escapedValue(4, 4, 8)).
                    let v = fdk_read_bits(&mut h_bs, 8) as u8;
                    if let Some(c) = config.get_mut(i) {
                        *c = v;
                    }
                }
                let build_up_status = self_.build_up_status;
                let terr = self_
                    .with_tp_cb(|tp, cb| {
                        transport_dec_in_band_config_cb(
                            tp,
                            &config,
                            config_length,
                            build_up_status,
                            &mut config_changed,
                            0,
                            &mut implicit_explicit_cfg_diff,
                            cb,
                        )
                    })
                    .unwrap_or(TRANSPORTDEC_OK);
                if terr != TRANSPORTDEC_OK {
                    error_status = AAC_DEC_PARSE_ERROR;
                    break 'bail;
                }
            }
        }

        /* For the first frame buildUpStatus is not set and no flushing is performed but
         * preroll AU's should processed. */
        /* For USAC there is no idle state. */
        if (self_.stream_info.num_channels == 0)
            && implicit_explicit_cfg_diff == 0
            && (self_.flags[0] & AC_USAC) != 0
        {
            self_.build_up_status = AACDEC_USAC_BUILD_UP_ON;
            /* sanity check: if buildUp status on -> flushing must be off */
            if self_.flush_status != AACDEC_FLUSH_OFF {
                self_.flush_status = AACDEC_FLUSH_OFF;
                error_status = AAC_DEC_PARSE_ERROR;
                break 'bail;
            }
        }

        if self_.flags[0] & AC_USAC != 0 {
            /* We are interested in preroll AUs if an explicit or an implicit config change is
             * signalized in other words if the build up status is set. */
            if self_.build_up_status == AACDEC_USAC_BUILD_UP_ON {
                let apply_crossfade = fdk_read_bit(&mut h_bs) as u8;
                if apply_crossfade != 0 {
                    self_.apply_crossfade |= AACDEC_CROSSFADE_BITMASK_PREROLL;
                } else {
                    self_.apply_crossfade &= !AACDEC_CROSSFADE_BITMASK_PREROLL;
                }
                fdk_read_bit(&mut h_bs); /* reserved */
                /* Read num preroll AU's */
                *num_preroll_au = escaped_value(&mut h_bs, 2, 4, 0);
                /* check limits for USAC */
                if *num_preroll_au > AACDEC_MAX_NUM_PREROLL_AU_USAC {
                    *num_preroll_au = 0;
                    error_status = AAC_DEC_PARSE_ERROR;
                    break 'bail;
                }
            }
        }

        let mut i: u32 = 0;
        while i < *num_preroll_au {
            let iu = i as usize;
            /* For every AU get length and offset in the bitstream */
            preroll_au_length[iu] = escaped_value(&mut h_bs, 16, 16, 0);
            if preroll_au_length[iu] > 0 {
                preroll_au_offset[iu] =
                    au_start_anchor.wrapping_sub(fdk_get_valid_bits(&mut h_bs) as i32) as u32;
                let independency_flag = fdk_read_bit(&mut h_bs);
                if i == 0 && independency_flag == 0 {
                    *num_preroll_au = 0;
                    error_status = AAC_DEC_PARSE_ERROR;
                    break 'bail;
                }
                fdk_push_for(&mut h_bs, preroll_au_length[iu].wrapping_mul(8).wrapping_sub(1));
                self_.preroll_au_length[iu] =
                    (preroll_au_length[iu].wrapping_mul(8)).wrapping_add(preroll_au_offset[iu]);
            } else {
                *num_preroll_au = 0;
                error_status = AAC_DEC_PARSE_ERROR; /* Something is wrong */
                break 'bail;
            }
            i += 1;
        }
    }

    /* bail: */
    if let Some(tp) = self_.h_input.as_deref_mut() {
        tp.bit_stream[0] = bs;
    }

    error_status
}

/// `CAacDecoder_ExtPayloadParse(self, hBs, count, previous_element, elIndex, fIsFillElement)`
/// (static in C): parse an extension payload.
pub fn caac_decoder_ext_payload_parse(
    self_: &mut AacDecoderInstance,
    h_bs: &mut FdkBitStream,
    count: &mut i32,
    previous_element: Mp4ElementId,
    el_index: i32,
    f_is_fill_element: i32,
) -> AacDecoderError {
    let mut error = AAC_DEC_OK;
    let mut bytes: i32 = (*count) >> 3;
    let mut crc_flag = 0;

    if *count < 4 {
        return AAC_DEC_PARSE_ERROR;
    } else if (fdk_get_valid_bits(h_bs) as i32) < *count {
        return AAC_DEC_DECODE_FRAME_ERROR;
    }

    let mut extension_type: ExtPayloadType = fdk_read_bits(h_bs, 4) as ExtPayloadType; /* bs_extension_type */
    *count -= 4;

    /* For ELD, the SBR signaling is explicit and parsed in aacDecoder_ParseExplicitMpsAndSbr(),
    therefore skip SBR if implicit present. */
    if (self_.flags[0] & AC_ELD) != 0
        && ((extension_type == EXT_SBR_DATA_CRC) || (extension_type == EXT_SBR_DATA))
    {
        extension_type = EXT_FIL; /* skip sbr data */
    }

    'bail: {
        match extension_type {
            EXT_DYNAMIC_RANGE => {
                let read_bits = aac_decoder_drc_mark_payload(
                    self_.h_drc_info.as_deref_mut(),
                    h_bs,
                    MPEG_DRC_EXT_DATA,
                );

                if read_bits > *count {
                    /* Read too much. Something went wrong! */
                    error = AAC_DEC_PARSE_ERROR;
                }
                *count -= read_bits;
            }
            EXT_LDSAC_DATA | EXT_SAC_DATA => {
                /* Read MPEG Surround Extension payload */
                if self_.flags[0] & AC_PS_PRESENT != 0 {
                    error = AAC_DEC_PARSE_ERROR;
                    break 'bail;
                }

                /* Handle SBR dual rate case */
                let (mps_sample_rate, mps_frame_size) = if self_.stream_info.ext_sampling_rate != 0
                {
                    (
                        self_.stream_info.ext_sampling_rate,
                        self_.stream_info.aac_samples_per_frame.wrapping_mul(
                            self_.stream_info.ext_sampling_rate
                                / nz(self_.stream_info.aac_sample_rate),
                        ),
                    )
                } else {
                    (self_.stream_info.aac_sample_rate, self_.stream_info.aac_samples_per_frame)
                };
                /* Setting of internal MPS state; may be reset in CAacDecoder_SyncQmfMode if
                decoder is unable to decode with user defined qmfMode */
                if self_.flags[0] & (AC_USAC | AC_RSVD50 | AC_ELD) == 0 {
                    self_.mps_enable_curr = self_.mps_enable_user;
                }
                if self_.mps_enable_curr != 0 {
                    if self_.qmf_domain.global_conf.qmf_domain_explicit_config == 0 {
                        /* if not done yet, allocate full MPEG Surround decoder instance */
                        if mps_is_full_instance_available(self_.p_mpeg_surround_decoder.as_deref())
                            == SAC_INSTANCE_NOT_FULL_AVAILABLE
                            && mpeg_surround_decoder_open(
                                &mut self_.p_mpeg_surround_decoder,
                                -1,
                                Some(&mut self_.qmf_domain),
                            ) != 0
                        {
                            return AAC_DEC_OUT_OF_MEMORY;
                        }
                    }
                    let err = mpeg_surround_decoder_parse(
                        self_.p_mpeg_surround_decoder.as_deref_mut(),
                        h_bs,
                        count,
                        self_.stream_info.aot,
                        mps_sample_rate,
                        mps_frame_size,
                        (self_.flags[0] & AC_INDEP) as i32,
                    );
                    if err == MPS_OK {
                        self_.flags[0] |= AC_MPS_PRESENT;
                    } else {
                        error = AAC_DEC_PARSE_ERROR;
                    }
                }
                /* Skip any trailing bytes */
                fdk_push_for(h_bs, *count as u32);
                *count = 0;
            }

            EXT_SBR_DATA_CRC | EXT_SBR_DATA => {
                if extension_type == EXT_SBR_DATA_CRC {
                    crc_flag = 1;
                }
                if is_channel_element(previous_element) {
                    let mut config_mode: u8 = 0;
                    let mut config_changed: u8 = 0;

                    caac_decoder_sync_qmf_mode(self_);

                    config_mode |= AC_CM_ALLOC_MEM;

                    let mut sbr_error = sbr_init_element(
                        self_.h_sbr_decoder.as_deref_mut(),
                        &mut self_.qmf_domain,
                        self_.stream_info.aac_sample_rate,
                        self_.stream_info.ext_sampling_rate,
                        self_.stream_info.aac_samples_per_frame,
                        self_.stream_info.aot,
                        previous_element,
                        el_index,
                        2, /* Signalize that harmonicSBR shall be ignored in the config change
                           detection */
                        0,
                        config_mode,
                        &mut config_changed,
                        self_.downscale_factor as i32,
                    );

                    if sbr_error == SBRDEC_OK {
                        let count_in = *count;
                        sbr_error = sbr_parse(
                            self_,
                            h_bs,
                            count,
                            count_in,
                            crc_flag,
                            previous_element,
                            el_index,
                        );
                        /* Enable SBR for implicit SBR signalling but only if no severe error
                         * happend. */
                        if (sbr_error == SBRDEC_OK) || (sbr_error == SBRDEC_PARSE_ERROR) {
                            self_.sbr_enabled = 1;
                        }
                    } else {
                        /* Do not try to apply SBR because initializing the element failed. */
                        self_.sbr_enabled = 0;
                    }
                    /* Citation from ISO/IEC 14496-3 chapter 4.5.2.1.5.2
                    Fill elements containing an extension_payload() with an extension_type of
                    EXT_SBR_DATA or EXT_SBR_DATA_CRC shall not contain any other
                    extension_payload of any other extension_type.
                    */
                    if f_is_fill_element != 0 {
                        fdk_push_bi_directional(h_bs, *count);
                        *count = 0;
                    } else {
                        /* If this is not a fill element with a known length, we are screwed and
                         * further parsing makes no sense. */
                        if sbr_error != SBRDEC_OK {
                            self_.frame_ok = 0;
                        }
                    }
                } else {
                    error = AAC_DEC_PARSE_ERROR;
                }
            }

            EXT_FILL_DATA => {
                let mut temp = fdk_read_bits(h_bs, 4) as i32;
                bytes -= 1;
                if temp != 0 {
                    error = AAC_DEC_PARSE_ERROR;
                } else {
                    while bytes > 0 {
                        temp = fdk_read_bits(h_bs, 8) as i32;
                        bytes -= 1;
                        if temp != 0xa5 {
                            error = AAC_DEC_PARSE_ERROR;
                            break;
                        }
                    }
                    *count = bytes << 3;
                }
            }

            EXT_DATA_ELEMENT => {
                let data_element_version = fdk_read_bits(h_bs, 4) as i32;
                *count -= 4;
                if data_element_version == 0
                /* ANC_DATA */
                {
                    let mut data_element_length: i32 = 0;
                    loop {
                        let temp = fdk_read_bits(h_bs, 8) as i32;
                        *count -= 8;
                        data_element_length += temp;
                        if temp != 255 {
                            break;
                        }
                    }

                    caac_decoder_anc_data_parse(&mut self_.anc_data, h_bs, data_element_length);
                    *count -= data_element_length << 3;
                } else {
                    /* align = 0 */
                    error = AAC_DEC_PARSE_ERROR;
                    break 'bail;
                }
            }

            _ => {
                if extension_type == EXT_DATA_LENGTH
                    && f_is_fill_element == 0 /* Makes no sens to have an additional length in a fill ... */
                    && (self_.flags[0] & AC_ER) != 0
                /* ... element because this extension payload type was ... */
                {
                    /* ... created to circumvent the missing length in ER-Syntax. */
                    let mut len = fdk_read_bits(h_bs, 4) as i32;
                    *count -= 4;

                    if len == 15 {
                        let add_len = fdk_read_bits(h_bs, 8) as i32;
                        *count -= 8;
                        len += add_len;

                        if add_len == 255 {
                            len += fdk_read_bits(h_bs, 16) as i32;
                            *count -= 16;
                        }
                    }
                    len <<= 3;
                    let mut bit_cnt = len;

                    if fdk_read_bits(h_bs, 4) as ExtPayloadType == EXT_DATA_LENGTH {
                        /* Check NOTE 2: The extension_payload() included here must not have
                         * extension_type == EXT_DATA_LENGTH. */
                        error = AAC_DEC_PARSE_ERROR;
                        break 'bail;
                    } else {
                        /* rewind and call myself again. */
                        fdk_push_back(h_bs, 4);

                        error = caac_decoder_ext_payload_parse(
                            self_,
                            h_bs,
                            &mut bit_cnt,
                            previous_element,
                            el_index,
                            1,
                        ); /* Treat same as fill element */

                        *count -= len - bit_cnt;
                    }
                    /* Note: the fall through in case the if statement above is not taken is
                     * intentional. */
                } else {
                    /* EXT_FIL and default: align = 4 */
                    fdk_push_for(h_bs, *count as u32);
                    *count = 0;
                }
            }
        }
    }

    /* bail: */
    if (error != AAC_DEC_OK) && f_is_fill_element != 0 {
        /* Skip the remaining extension bytes */
        fdk_push_bi_directional(h_bs, *count);
        *count = 0;
        /* Patch error code because decoding can go on. */
        error = AAC_DEC_OK;
        /* Be sure that parsing errors have been stored. */
    }
    error
}

/// A divisor of the C code that is never 0 in the library (a sample rate of a configured
/// decoder); the port divides by 1 instead of trapping.
#[inline]
pub(crate) fn nz(x: i32) -> i32 {
    if x == 0 {
        1
    } else {
        x
    }
}

/// `sbrDecoder_Parse(self->hSbrDecoder, hBs, self->pDrmBsBuffer, self->drmBsBufferSize, count,
/// bsPayLen, crcFlag, prevElement, elementIndex, self->flags[0], self->elFlags)`: the SBR
/// decoder writes the reversed DRM payload into `pDrmBsBuffer` (the `DrmBsBuffer` view of
/// `pWorkBufferCore1`, `drmBsBufferSize` bytes; empty without DRM, where C passes `NULL`).
#[allow(clippy::too_many_arguments)]
pub(crate) fn sbr_parse(
    self_: &mut AacDecoderInstance,
    h_bs: &mut FdkBitStream,
    count: &mut i32,
    bs_pay_len: i32,
    crc_flag: i32,
    prev_element: Mp4ElementId,
    element_index: i32,
) -> SbrError {
    let AacDecoderInstance {
        h_sbr_decoder,
        p_work_buffer_core1,
        p_drm_bs_buffer,
        drm_bs_buffer_size,
        flags,
        el_flags,
        ..
    } = self_;
    let Some(sbr) = h_sbr_decoder.as_deref_mut() else {
        // (None: the reference has no defined result here)
        return SBRDEC_NOT_INITIALIZED;
    };
    let drm: &mut [u8] = match p_work_buffer_core1.as_deref_mut() {
        Some(wb) if *p_drm_bs_buffer => {
            let b = wb.drm_bs_buffer();
            let n = (*drm_bs_buffer_size as usize).min(b.len());
            &mut b[..n]
        }
        _ => &mut [],
    };
    sbr_decoder_parse(
        sbr,
        h_bs,
        drm,
        count,
        bs_pay_len,
        crc_flag,
        prev_element,
        element_index,
        flags[0],
        el_flags,
    )
}

/// `sbrDecoder_InitElement(self->hSbrDecoder, ...)` with the decoder's QMF domain
/// (`SBRDEC_NOT_INITIALIZED` for a `NULL` handle, as in C).
#[allow(clippy::too_many_arguments)]
pub(crate) fn sbr_init_element(
    h_sbr_decoder: Option<&mut SbrDecoderInstance>,
    qd: &mut FdkQmfDomain,
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
) -> SbrError {
    let Some(sbr) = h_sbr_decoder else {
        return SBRDEC_NOT_INITIALIZED;
    };
    sbr_decoder_init_element(
        sbr,
        qd,
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
    )
}

/// `mpegSurroundDecoder_IsFullMpegSurroundDecoderInstanceAvailable(pMpegSurroundDecoder)`
/// (`None`: not available).
pub(crate) fn mps_is_full_instance_available(d: Option<&CMpegSurroundDecoder>) -> i32 {
    d.map_or(SAC_INSTANCE_NOT_FULL_AVAILABLE, |d| {
        mpeg_surround_decoder_is_full_mpeg_surround_decoder_instance_available(d)
    })
}

/// `aacDecoder_ParseExplicitMpsAndSbr(self, bs, previous_element, previous_element_index,
/// element_index, el_cnt[])` (static in C)
pub fn aac_decoder_parse_explicit_mps_and_sbr(
    self_: &mut AacDecoderInstance,
    bs: &mut FdkBitStream,
    previous_element: Mp4ElementId,
    previous_element_index: i32,
    element_index: i32,
    el_cnt: &[i32],
) -> AacDecoderError {
    let mut error_status = AAC_DEC_OK;

    /* get the remaining bits of this frame */
    let mut bit_cnt = self_.with_tp(bs, |tp| transport_dec_get_au_bits_remaining(tp, 0));

    if (self_.flags[0] & AC_SBR_PRESENT) != 0
        && (self_.flags[0] & (AC_USAC | AC_RSVD50 | AC_ELD | AC_DRM)) != 0
    {
        let mut err = SBRDEC_OK;
        let num_ch_elements = el_cnt[ID_SCE as usize]
            + el_cnt[ID_CPE as usize]
            + el_cnt[ID_LFE as usize]
            + el_cnt[ID_USAC_SCE as usize]
            + el_cnt[ID_USAC_CPE as usize]
            + el_cnt[ID_USAC_LFE as usize];
        let mut bit_cnt_tmp = bit_cnt;

        let mut ch_el_idx = if self_.flags[0] & AC_USAC != 0 {
            num_ch_elements - 1
        } else {
            0 /* ELD case */
        };

        while ch_el_idx < num_ch_elements {
            let ei = element_index.clamp(0, MAX_ELEMENTS as i32 - 1) as usize;
            let sbr_type: Mp4ElementId = if self_.flags[0] & AC_USAC != 0 {
                fdk_assert!(
                    (self_.elements[ei] == ID_USAC_SCE) || (self_.elements[ei] == ID_USAC_CPE)
                );
                if is_stereo_sbr(self_.elements[ei], self_.usac_stereo_config_index[ei]) {
                    ID_CPE
                } else {
                    ID_SCE
                }
            } else {
                self_.elements.get(ch_el_idx as usize).copied().unwrap_or(ID_NONE)
            };
            let crc_flag = (self_.flags[0] & AC_SBRCRC) as i32;
            let err_tmp = sbr_parse(self_, bs, &mut bit_cnt, -1, crc_flag, sbr_type, ch_el_idx);
            if err_tmp != SBRDEC_OK {
                err = err_tmp;
                bit_cnt_tmp = bit_cnt;
                bit_cnt = 0;
            }
            ch_el_idx += 1;
        }
        match err {
            SBRDEC_PARSE_ERROR => {
                /* Can not go on parsing because we do not know the length of the SBR extension
                data. */
                fdk_push_for(bs, bit_cnt_tmp as u32);
                bit_cnt = 0;
            }
            SBRDEC_OK => {
                self_.sbr_enabled = 1;
            }
            _ => {
                self_.frame_ok = 0;
            }
        }
    }

    if (bit_cnt > 0) && (self_.flags[0] & (AC_USAC | AC_RSVD50)) != 0 {
        let ei = element_index.clamp(0, MAX_ELEMENTS as i32 - 1) as usize;
        if (self_.flags[0] & AC_MPS_PRESENT) != 0 || (self_.el_flags[ei] & AC_EL_USAC_MPS212) != 0 {
            let err = mpeg_surround_decoder_parse_no_header(
                self_.p_mpeg_surround_decoder.as_deref_mut(),
                Some(bs),
                &mut bit_cnt,
                (self_.flags[0] & AC_INDEP) as i32,
            );
            if err != MPS_OK {
                self_.frame_ok = 0;
                error_status = AAC_DEC_PARSE_ERROR;
            }
        }
    }

    if self_.flags[0] & AC_DRM != 0 {
        bit_cnt = fdk_get_valid_bits(bs) as i32;
        if bit_cnt != 0 {
            fdk_push_bi_directional(bs, bit_cnt);
        }
    }

    if self_.flags[0] & (AC_USAC | AC_RSVD50 | AC_DRM) == 0 {
        while bit_cnt > 7 {
            error_status = caac_decoder_ext_payload_parse(
                self_,
                bs,
                &mut bit_cnt,
                previous_element,
                previous_element_index,
                0,
            );
            if error_status != AAC_DEC_OK {
                self_.frame_ok = 0;
                error_status = AAC_DEC_PARSE_ERROR;
                break;
            }
        }
    }
    error_status
}

/// `CStreamInfoInit(pStreamInfo)` (static in C): stream configuration and information.
pub fn cstream_info_init(p_stream_info: &mut CStreamInfo) {
    p_stream_info.aac_sample_rate = 0;
    p_stream_info.profile = -1;
    p_stream_info.aot = AOT_NONE;

    p_stream_info.channel_config = -1;
    p_stream_info.bit_rate = 0;
    p_stream_info.aac_samples_per_frame = 0;

    p_stream_info.ext_aot = AOT_NONE;
    p_stream_info.ext_sampling_rate = 0;

    p_stream_info.flags = 0;

    p_stream_info.ep_config = -1; /* default: no ER */

    p_stream_info.num_channels = 0;
    p_stream_info.sample_rate = 0;
    p_stream_info.frame_size = 0;

    p_stream_info.output_delay = 0;

    /* DRC */
    p_stream_info.drc_prog_ref_lev = -1; /* set program reference level to not indicated */
    p_stream_info.drc_pres_mode = -1; /* default: presentation mode not indicated */

    p_stream_info.output_loudness = -1; /* default: no loudness metadata present */
}

/// `CAacDecoder_Open(bsFormat)`: initialization of the AAC decoder instance.
pub fn caac_decoder_open(_bs_format: TransportType) -> Option<Box<AacDecoderInstance>> {
    let mut self_ = AacDecoderInstance::zeroed();

    fdk_qmf_domain_clear_requested(&mut self_.qmf_domain.global_conf);

    /* Assign channel mapping info arrays (doing so removes dependency of settings header in
     * API header). */
    // streamInfo.pChannelIndices and pChannelType point to channelIndices and channelType; the
    // port keeps these arrays inside CStreamInfo (see the module documentation).
    self_.downscale_factor = 1;
    self_.downscale_factor_in_bs = 1;

    /* initialize anc data */
    caac_decoder_anc_data_init(&mut self_.anc_data, AncBuffer::None, 0);

    /* initialize stream info */
    cstream_info_init(&mut self_.stream_info);

    /* initialize progam config */
    cprogram_config_init(&mut self_.pce);

    /* initialize error concealment common data */
    cconcealment_init_common_data(Some(&mut self_.conceal_common_data));
    self_.conceal_method_user = ConcealMethodNone; /* undefined -> auto mode */

    self_.h_drc_info = Some(get_drc_info(0));
    /* Init common DRC structure */
    aac_decoder_drc_init(self_.h_drc_info.as_deref_mut());
    /* Set default frame delay */
    aac_decoder_drc_set_param(
        self_.h_drc_info.as_deref_mut(),
        DRC_BS_DELAY,
        cconcealment_get_delay(Some(&self_.conceal_common_data)) as i32,
    );
    self_.work_buffer_core1 = Some(get_work_buffer_core1(0));

    self_.work_buffer_core2 = Some(get_work_buffer_core2(0));

    /* When RSVD60 is active use dedicated memory for core decoding */
    self_.p_time_data2 = Some(get_work_buffer_core5(0));
    self_.time_data2_size = get_required_mem_work_buffer_core5() as i32;

    Some(self_)
}

/// `CAacDecoder_DeInit(self, subStreamIndex)` (static in C): revert `CAacDecoder_Init()`.
pub fn caac_decoder_de_init(self_: &mut AacDecoderInstance, sub_stream_index: i32) {
    self_.d_gen = self_.d_gen.wrapping_add(1);
    self_.asc_channels[0] = 0;
    self_.elements[0] = ID_END;

    for ch in 0..NCH {
        // pComStaticData (with its pWorkBufferCore1 for the first channel and its
        // cplxPredictionData), pComData, the overlap buffer, hArCo and the channel infos.
        self_.p_com_static_data[ch] = None;
        self_.p_com_data[ch] = None;
        self_.p_aac_decoder_static_channel_info[ch] = None;
        self_.p_aac_decoder_channel_info[ch] = None;
    }
    self_.com_static_data.clear();
    self_.com_data.clear();
    self_.p_work_buffer_core1 = None;

    for el in 0..MAX_ELEMENTS {
        self_.cpe_static_data[el] = None;
    }

    fdk_delay_destroy(&mut self_.usac_residual_delay);

    self_.aac_channels = 0;
    self_.stream_info.aac_sample_rate = 0;
    self_.stream_info.sample_rate = 0;
    /* This samplerate value is checked for configuration change, not the others above. */
    if let Some(sri) = self_.sampling_rate_info.get_mut(sub_stream_index as usize) {
        sri.sampling_rate = 0;
    }
}

/// `CAacDecoder_AcceptFlags(self, asc, flags, elFlags, streamIndex, elementOffset)` (static in
/// C): accept flags and element flags.
pub fn caac_decoder_accept_flags(
    self_: &mut AacDecoderInstance,
    flags: u32,
    el_flags: &[u32; MAX_ELEMENTS],
    stream_index: usize,
) {
    self_.el_flags = *el_flags;

    self_.flags[stream_index] = flags;
}

/// `CAacDecoder_CtrlCFGChange(self, flushStatus, flushCnt, buildUpStatus, buildUpCnt)`: set the
/// config change parameters.
pub fn caac_decoder_ctrl_cfg_change(
    self_: &mut AacDecoderInstance,
    flush_status: u8,
    flush_cnt: i8,
    build_up_status: u8,
    build_up_cnt: i8,
) -> AacDecoderError {
    self_.flush_status = flush_status;
    self_.flush_cnt = flush_cnt;
    self_.build_up_status = build_up_status;
    self_.build_up_cnt = build_up_cnt;

    AAC_DEC_OK
}

/// `CAacDecoder_FreeMem(self, subStreamIndex)`: free the config dependent AAC memory.
pub fn caac_decoder_free_mem(
    self_: &mut AacDecoderInstance,
    sub_stream_index: i32,
) -> AacDecoderError {
    caac_decoder_de_init(self_, sub_stream_index);

    AAC_DEC_OK
}

/// `CAacDecoder_Close(self)`: destroy the AAC decoder.
pub fn caac_decoder_close(mut self_: Box<AacDecoderInstance>) {
    caac_decoder_de_init(&mut self_, 0);

    for ch in 0..8 {
        self_.p_time_data_flush[ch] = None;
    }

    self_.h_drc_info = None;
    self_.work_buffer_core1 = None;
    /* Free WorkBufferCore2 */
    self_.work_buffer_core2 = None;
    self_.p_time_data2 = None;

    fdk_aac_rs_fdk::qmf_domain::fdk_qmf_domain_close(&mut self_.qmf_domain);
}

/// Allocates the per element memory of channel `ch` (`pComStaticData`, `pComData`) as
/// `CAacDecoder_Init()` does. Returns the indices of the new allocations.
fn alloc_element_memory(self_: &mut AacDecoderInstance, ch: usize, first: bool) -> (usize, usize) {
    self_.com_static_data.push(Box::default());
    let cs = self_.com_static_data.len() - 1;
    if first {
        self_.com_data.push(get_work_buffer_core6(0));
        self_.p_work_buffer_core1 = Some(get_work_buffer_core1(0));
    } else {
        self_.com_data.push(Box::default());
    }
    let cd = self_.com_data.len() - 1;
    self_.p_com_static_data[ch] = Some(cs);
    self_.p_com_data[ch] = Some(cd);
    (cs, cd)
}

/// `CAacDecoder_Init(self, asc, configMode, configChanged)`: initialization of the channel
/// elements for the configuration `asc`.
pub fn caac_decoder_init(
    self_: &mut AacDecoderInstance,
    asc: &CSAudioSpecificConfig,
    config_mode: u8,
    config_changed: &mut u8,
) -> AacDecoderError {
    let err = AAC_DEC_OK;
    let mut asc_channels: i32;
    let mut asc_changed = 0;
    let init_render_mode;
    let mut usac_stereo_config_index: i8 = -1;
    let mut usac_residual_delay_comp_samples: i32 = 0;
    let stream_index = 0usize;
    let mut flush_channels: i32 = 0;

    let mut flags: u32;
    let mut el_flags = [0u32; MAX_ELEMENTS];

    let mut sbr_enabled = self_.sbr_enabled;
    let sbr_enabled_prev;
    let mut mps_enable_curr = self_.mps_enable_curr;

    let mut downscale_factor = self_.downscale_factor;
    let mut downscale_factor_in_bs = self_.downscale_factor_in_bs;

    self_.aac_out_data_headroom = 3;

    // set profile and check for supported aot
    // leave profile on default (=-1) for all other supported MPEG-4 aot's except aot=2 (=AAC-LC)
    match asc.m_aot {
        AOT_AAC_LC | AOT_ER_AAC_SCAL | AOT_SBR | AOT_PS | AOT_ER_AAC_LC | AOT_ER_AAC_LD
        | AOT_DRM_AAC | AOT_DRM_SURROUND => {
            if asc.m_aot == AOT_AAC_LC {
                self_.stream_info.profile = 1;
            }
            if (asc.m_aot == AOT_AAC_LC || asc.m_aot == AOT_ER_AAC_SCAL)
                && asc.m_sc.m_ga_specific_config.m_layer > 0
            {
                /* aac_scalable_extension_element() currently not supported. */
                return AAC_DEC_UNSUPPORTED_FORMAT;
            }
            init_render_mode = AACDEC_RENDER_IMDCT;
        }
        AOT_ER_AAC_ELD => {
            init_render_mode = AACDEC_RENDER_ELDFB;
        }
        AOT_USAC => {
            init_render_mode = AACDEC_RENDER_IMDCT;
        }
        _ => return AAC_DEC_UNSUPPORTED_AOT,
    }

    if cprogram_config_is_valid(&self_.pce) != 0 && (asc.m_channel_configuration > 0) {
        /* Compare the stored (old) PCE with a default PCE created from the (new) channel_config
        (on a temporal buffer) to find out wheter we can keep it (and its metadata) or not. */
        let mut tmp_pce = CProgramConfig::zeroed();

        cprogram_config_get_default(&mut tmp_pce, asc.m_channel_configuration as u32);
        let pce_cmp_result = cprogram_config_compare(&self_.pce, &tmp_pce);
        if (pce_cmp_result < 0) /* Reset if PCEs are completely different ... */
            || (pce_cmp_result > 1)
        {
            /*            ... or have a different layout. */
            cprogram_config_init(&mut self_.pce);
        } /* Otherwise keep the PCE (and its metadata). */
    } else {
        cprogram_config_init(&mut self_.pce);
    }

    /* set channels */
    match asc.m_channel_configuration {
        0 => match asc.m_aot {
            AOT_USAC => {
                self_.ch_map_index = 0;
                asc_channels = asc.m_sc.m_usac_config.m_n_usac_channels as i32;
            }
            _ => {
                /* get channels from program config (ASC) */
                if cprogram_config_is_valid(&asc.m_progr_config_element) != 0 {
                    asc_channels = asc.m_progr_config_element.num_channels as i32;
                    if asc_channels > 0 {
                        /* valid number of channels -> copy program config element (PCE) from
                         * ASC */
                        self_.pce = asc.m_progr_config_element;
                        /* Built element table */
                        let mut el_tmp = cprogram_config_get_element_table(
                            &asc.m_progr_config_element,
                            &mut self_.elements,
                            (8 + 8) as i32,
                            &mut self_.ch_map_index,
                        );
                        while (el_tmp as usize) < MAX_ELEMENTS {
                            self_.elements[el_tmp.max(0) as usize] = ID_NONE;
                            el_tmp += 1;
                        }
                    } else {
                        return AAC_DEC_UNSUPPORTED_CHANNELCONFIG;
                    }
                } else {
                    self_.ch_map_index = 0;
                    return AAC_DEC_UNSUPPORTED_CHANNELCONFIG;
                }
            }
        },
        1..=6 => {
            asc_channels = asc.m_channel_configuration as i32;
        }
        11 => {
            asc_channels = 7;
        }
        7 | 12 | 14 => {
            asc_channels = 8;
        }
        _ => return AAC_DEC_UNSUPPORTED_CHANNELCONFIG,
    }

    if asc.m_aot == AOT_USAC {
        flush_channels = asc_channels.min(8);
        let mut num_channel: i32 = 0;
        pcm_dmx_get_param(
            self_.h_pcm_utils.as_deref(),
            MIN_NUMBER_OF_OUTPUT_CHANNELS,
            Some(&mut num_channel),
        );
        flush_channels = num_channel.max(flush_channels).min(8);
    }

    if is_usac(asc.m_aot) {
        let uc = &asc.m_sc.m_usac_config;
        for el in 0..uc.m_usac_num_elements.min(TP_USAC_MAX_ELEMENTS as u32) as usize {
            /* fix number of core channels aka ascChannels for stereoConfigIndex = 1 cases */
            if uc.element[el].m_stereo_config_index == 1 {
                asc_channels -= 1; /* stereoConfigIndex == 1 stereo cases do actually contain
                                   only a mono core channel. */
            } else if uc.element[el].m_stereo_config_index == 2 {
                /* In this case it is necessary to follow up the DMX signal delay caused by HBE
                   also with the residual signal (2nd core channel). The SBR overlap delay is
                   not regarded here, this is handled by the MPS212 implementation.
                */
                if uc.element[el].m_harmonic_sbr != 0 {
                    usac_residual_delay_comp_samples = usac_residual_delay_comp_samples
                        .wrapping_add(asc.m_samples_per_frame as i32);
                }
                if uc.m_core_sbr_frame_length_index == 4 {
                    usac_residual_delay_comp_samples += 6 * 16; /* difference between 12 SBR
                                                                overlap slots from SBR and 6
                                                                slots delayed in MPS212 */
                }
            }
        }
    }

    let aac_channels_offset: usize = 0;
    let aac_channels_offset_idx: i32 = 0;
    let element_offset: usize = 0;
    if (asc_channels <= 0)
        || (asc_channels > 8)
        || (asc.m_channel_configuration as i32 > AACDEC_MAX_CH_CONF)
    {
        return AAC_DEC_UNSUPPORTED_CHANNELCONFIG;
    }

    /* Set syntax flags */
    flags = 0;
    el_flags.fill(0);

    'bail: {
        if (asc.m_channel_configuration > 0) || is_usac(asc.m_aot) {
            if is_usac(asc.m_aot) {
                /* copy pointer to usac config (this is preliminary since there's an ongoing
                discussion about storing the config-part of the bitstream rather than the
                complete decoded configuration) */
                self_.p_usac_config[stream_index] = true;
                let uc = &asc.m_sc.m_usac_config;

                /* copy list of elements */
                if uc.m_usac_num_elements > MAX_ELEMENTS as u32 {
                    break 'bail;
                }

                if self_.num_usac_elements[stream_index] != uc.m_usac_num_elements {
                    asc_changed = 1;
                }

                if config_mode & AC_CM_ALLOC_MEM != 0 {
                    self_.num_usac_elements[stream_index] = uc.m_usac_num_elements;
                }

                mps_enable_curr = 0;
                for el_ in 0..uc.m_usac_num_elements as usize {
                    let el = el_ + element_offset;
                    // The configuration holds TP_USAC_MAX_ELEMENTS entries; m_usacNumElements
                    // is limited by the parser.
                    let ue = uc.element.get(el_).copied().unwrap_or_default();
                    if self_.elements[el] != ue.usac_element_type {
                        asc_changed = 1;
                    }
                    if self_.usac_stereo_config_index[el] != ue.m_stereo_config_index {
                        asc_changed = 1;
                    }
                    if config_mode & AC_CM_ALLOC_MEM != 0 {
                        self_.elements[el] = ue.usac_element_type;
                        /* for Unified Stereo Coding */
                        self_.usac_stereo_config_index[el] = ue.m_stereo_config_index;
                        if self_.elements[el] == ID_USAC_CPE {
                            mps_enable_curr |= (self_.usac_stereo_config_index[el] != 0) as u8;
                        }
                    }

                    el_flags[el] |= if ue.m_noise_filling != 0 { AC_EL_USAC_NOISE } else { 0 };
                    el_flags[el] |=
                        if ue.m_stereo_config_index > 0 { AC_EL_USAC_MPS212 } else { 0 };
                    el_flags[el] |= if ue.m_inter_tes != 0 { AC_EL_USAC_ITES } else { 0 };
                    el_flags[el] |= if ue.m_pvc != 0 { AC_EL_USAC_PVC } else { 0 };
                    el_flags[el] |=
                        if ue.usac_element_type == ID_USAC_LFE { AC_EL_USAC_LFE } else { 0 };
                    el_flags[el] |= if ue.usac_element_type == ID_USAC_LFE { AC_EL_LFE } else { 0 };
                    if (ue.usac_element_type == ID_USAC_CPE)
                        && (self_.usac_stereo_config_index[el] == 0)
                    {
                        el_flags[el] |= AC_EL_USAC_CP_POSSIBLE;
                    }
                }

                self_.has_audio_pre_roll = 0;
                if uc.m_usac_num_elements != 0 {
                    self_.has_audio_pre_roll =
                        uc.element[0].ext_element.usac_ext_element_has_audio_pre_roll;
                }
                if config_mode & AC_CM_ALLOC_MEM != 0 {
                    let idx = element_offset + uc.m_usac_num_elements as usize;
                    if let Some(e) = self_.elements.get_mut(idx) {
                        *e = ID_END;
                    }
                }
            } else {
                /* Initialize constant mappings for channel config 1-7 */
                let row = &ELEMENTS_TAB[(asc.m_channel_configuration - 1) as usize];
                let mut i = 0usize;
                while i < AACDEC_CH_ELEMENTS_TAB_SIZE as usize {
                    self_.elements[i] = row[i];
                    i += 1;
                }
                while i < MAX_ELEMENTS {
                    self_.elements[i] = ID_NONE;
                    i += 1;
                }
            }

            {
                let mut ch = 0usize;
                while (ch as i32) < asc_channels {
                    self_.ch_mapping[ch] = ch as u8;
                    ch += 1;
                }
                while ch < 8 {
                    self_.ch_mapping[ch] = 255;
                    ch += 1;
                }
            }

            self_.ch_map_index = asc.m_channel_configuration as u8;
        } else if cprogram_config_is_valid(&asc.m_progr_config_element) != 0 {
            /* Set matrix mixdown infos if available from PCE. */
            pcm_dmx_set_matrix_mixdown_from_pce(
                self_.h_pcm_utils.as_deref_mut(),
                asc.m_progr_config_element.matrix_mixdown_index_present as i32,
                asc.m_progr_config_element.matrix_mixdown_index as i32,
                asc.m_progr_config_element.pseudo_surround_enable as i32,
            );
        }

        self_.stream_info.channel_config = asc.m_channel_configuration as i32;

        if self_.stream_info.aot != asc.m_aot {
            if config_mode & AC_CM_ALLOC_MEM != 0 {
                self_.stream_info.aot = asc.m_aot;
            }
            asc_changed = 1;
        }

        if asc.m_aot == AOT_ER_AAC_ELD
            && asc.m_sc.m_eld_specific_config.m_downscaled_sampling_frequency != 0
        {
            let dsf = asc.m_sc.m_eld_specific_config.m_downscaled_sampling_frequency;
            if self_.sampling_rate_info[0].sampling_rate != dsf
                || self_.sampling_rate_info[0]
                    .sampling_rate
                    .wrapping_mul(self_.downscale_factor as u32)
                    != asc.m_sampling_frequency
            {
                /* get downscaledSamplingFrequency from ESC and compute the downscale factor */
                downscale_factor_in_bs = (asc.m_sampling_frequency / dsf) as u8;
                if (downscale_factor_in_bs == 1
                    || downscale_factor_in_bs == 2
                    || (downscale_factor_in_bs == 3
                        && asc.m_sc.m_eld_specific_config.m_frame_length_flag != 0)
                    || downscale_factor_in_bs == 4)
                    && ((asc.m_sampling_frequency % dsf) == 0)
                {
                    downscale_factor = downscale_factor_in_bs;
                } else {
                    downscale_factor_in_bs = 1;
                    downscale_factor = 1;
                }
            }
        } else {
            downscale_factor_in_bs = 1;
            downscale_factor = 1;
        }

        if self_.downscale_factor_in_bs != downscale_factor_in_bs {
            if config_mode & AC_CM_ALLOC_MEM != 0 {
                self_.downscale_factor_in_bs = downscale_factor_in_bs;
                self_.downscale_factor = downscale_factor;
            }
            asc_changed = 1;
        }

        // downscaleFactor is 1..4 here (the stored factor is never 0 after CAacDecoder_Open()).
        if (asc.m_samples_per_frame as i32) % (downscale_factor as i32).max(1) != 0 {
            return AAC_DEC_UNSUPPORTED_SAMPLINGRATE; /* frameSize/dsf must be an integer number */
        }

        self_.stream_info.bit_rate = 0;

        if asc.m_aot == AOT_ER_AAC_ELD {
            if self_.use_ld_qmf_time_align != asc.m_sc.m_eld_specific_config.m_use_ld_qmf_time_align
            {
                asc_changed = 1;
            }
            if config_mode & AC_CM_ALLOC_MEM != 0 {
                self_.use_ld_qmf_time_align =
                    asc.m_sc.m_eld_specific_config.m_use_ld_qmf_time_align;
            }
            if sbr_enabled as i32 != asc.m_sbr_present_flag as i32 {
                asc_changed = 1;
            }
        }

        self_.stream_info.ext_aot = asc.m_extension_audio_object_type;
        if self_.stream_info.ext_sampling_rate != asc.m_extension_sampling_frequency as i32 {
            asc_changed = 1;
        }
        if config_mode & AC_CM_ALLOC_MEM != 0 {
            self_.stream_info.ext_sampling_rate = asc.m_extension_sampling_frequency as i32;
        }
        flags |= if asc.m_sbr_present_flag != 0 { AC_SBR_PRESENT } else { 0 };
        flags |= if asc.m_ps_present_flag != 0 { AC_PS_PRESENT } else { 0 };
        if asc.m_sbr_present_flag != 0 {
            sbr_enabled = 1;
            sbr_enabled_prev = 1;
        } else {
            sbr_enabled = 0;
            sbr_enabled_prev = 0;
        }
        if sbr_enabled != 0 && asc.m_extension_sampling_frequency != 0 {
            if downscale_factor != 1 && (downscale_factor & 1) != 0 {
                return AAC_DEC_UNSUPPORTED_SAMPLINGRATE; /* SBR needs an even downscale factor */
            }
            if config_mode & AC_CM_ALLOC_MEM != 0 {
                self_.stream_info.ext_sampling_rate /= (self_.downscale_factor as i32).max(1);
            }
        }
        if (asc.m_aot == AOT_AAC_LC)
            && (asc.m_sbr_present_flag == 1)
            && (asc.m_extension_sampling_frequency > asc.m_sampling_frequency.wrapping_mul(2))
        {
            return AAC_DEC_UNSUPPORTED_SAMPLINGRATE; /* Core decoder supports at most a 1:2
                                                     upsampling for HE-AAC and HE-AACv2 */
        }

        /* --------- vcb11 ------------ */
        flags |= if asc.m_vcb11_flag != 0 { AC_ER_VCB11 } else { 0 };

        /* ---------- rvlc ------------ */
        flags |= if asc.m_rvlc_flag != 0 { AC_ER_RVLC } else { 0 };

        /* ----------- hcr ------------ */
        flags |= if asc.m_hcr_flag != 0 { AC_ER_HCR } else { 0 };

        if asc.m_aot == AOT_ER_AAC_ELD {
            mps_enable_curr = 0;
            flags |= AC_ELD;
            flags |= if asc.m_sbr_present_flag != 0 { AC_SBR_PRESENT } else { 0 }; /* Need to set
                                                                                   the SBR flag
                                                                                   for backward
                                                                                   compatibility
                                                                                   reasons. Even
                                                                                   if SBR is not
                                                                                   supported. */
            flags |= if asc.m_sc.m_eld_specific_config.m_sbr_crc_flag != 0 { AC_SBRCRC } else { 0 };
            flags |= if asc.m_sc.m_eld_specific_config.m_use_ld_qmf_time_align != 0 {
                AC_MPS_PRESENT
            } else {
                0
            };
            if self_.mps_applicable != 0 {
                mps_enable_curr = asc.m_sc.m_eld_specific_config.m_use_ld_qmf_time_align;
            }
        }
        flags |= if asc.m_aot == AOT_ER_AAC_LD { AC_LD } else { 0 };
        flags |= if asc.m_ep_config >= 0 { AC_ER } else { 0 };

        if asc.m_aot == AOT_USAC {
            flags |= AC_USAC;
            flags |= if asc.m_sc.m_usac_config.element[0].m_stereo_config_index > 0 {
                AC_MPS_PRESENT
            } else {
                0
            };
        }
        if asc.m_aot == AOT_DRM_AAC {
            flags |= AC_DRM | AC_SBRCRC | AC_SCALABLE;
        }
        if asc.m_aot == AOT_DRM_SURROUND {
            flags |= AC_DRM | AC_SBRCRC | AC_SCALABLE | AC_MPS_PRESENT;
            fdk_assert!(asc.m_ps_present_flag == 0);
        }
        if (asc.m_aot == AOT_AAC_SCAL) || (asc.m_aot == AOT_ER_AAC_SCAL) {
            flags |= AC_SCALABLE;
        }

        if (asc.m_ep_config >= 0) && (asc.m_channel_configuration <= 0) {
            /* we have to know the number of channels otherwise no decoding is possible */
            return AAC_DEC_UNSUPPORTED_ER_FORMAT;
        }

        self_.stream_info.ep_config = asc.m_ep_config;
        /* self->hInput->asc.m_epConfig = asc->m_epConfig; */

        if asc.m_ep_config > 1 {
            return AAC_DEC_UNSUPPORTED_ER_FORMAT;
        }

        /* Check if samplerate changed. */
        if (self_.sampling_rate_info[stream_index].sampling_rate != asc.m_sampling_frequency)
            || (self_.stream_info.aac_samples_per_frame
                != (asc.m_samples_per_frame as i32) / (downscale_factor as i32).max(1))
        {
            asc_changed = 1;

            if config_mode & AC_CM_ALLOC_MEM != 0 {
                /* Update samplerate info. */
                let error = get_sampling_rate_info(
                    &mut self_.sampling_rate_info[stream_index],
                    asc.m_samples_per_frame,
                    asc.m_sampling_frequency_index as u32,
                    asc.m_sampling_frequency,
                );
                if error != AAC_DEC_OK {
                    return error;
                }
                let dsf = (self_.downscale_factor as u32).max(1);
                self_.stream_info.aac_sample_rate =
                    (self_.sampling_rate_info[0].sampling_rate / dsf) as i32;
                self_.stream_info.aac_samples_per_frame = (asc.m_samples_per_frame / dsf) as i32;
                if self_.stream_info.aac_sample_rate <= 0 {
                    return AAC_DEC_UNSUPPORTED_SAMPLINGRATE;
                }
            }
        }

        /* Check if amount of channels has changed. */
        if self_.asc_channels[stream_index] != asc_channels {
            asc_changed = 1;
        }

        /* detect config change */
        if config_mode & AC_CM_DET_CFG_CHANGE != 0 {
            if asc_changed != 0 {
                *config_changed = 1;
            }

            caac_decoder_accept_flags(self_, flags, &el_flags, stream_index);

            return err;
        }

        /* set AC_USAC_SCFGI3 globally if any usac element uses */
        if asc.m_aot == AOT_USAC && sbr_enabled != 0 {
            let n = asc.m_sc.m_usac_config.m_usac_num_elements.min(MAX_ELEMENTS as u32) as usize;
            for el_ in 0..n {
                let el = element_offset + el_;
                if is_usac_channel_element(self_.elements[el]) {
                    if usac_stereo_config_index < 0 {
                        usac_stereo_config_index = self_.usac_stereo_config_index[el] as i8;
                    } else if (usac_stereo_config_index as i32
                        != self_.usac_stereo_config_index[el] as i32)
                        || (self_.usac_stereo_config_index[el] > 0)
                    {
                        break 'bail;
                    }
                }
            }

            if usac_stereo_config_index < 0 {
                break 'bail;
            }

            if usac_stereo_config_index == 3 {
                flags |= AC_USAC_SCFGI3;
            }
        }

        if *config_changed != 0 {
            self_.d_gen = self_.d_gen.wrapping_add(1);
            /* Set up QMF domain for AOTs with explicit signalling of SBR and or MPS. This is to
               be able to play out the first frame alway with the correct frame size and
               sampling rate even in case of concealment.
            */
            match asc.m_aot {
                AOT_USAC => {
                    if sbr_enabled != 0 {
                        let sri = asc.m_sc.m_usac_config.m_sbr_ratio_index;
                        fdk_assert!(sri > 0);
                        fdk_assert!(stream_index == 0);

                        let gc = &mut self_.qmf_domain.global_conf;
                        gc.n_input_channels_requested = asc_channels as u8;
                        gc.n_output_channels_requested =
                            if usac_stereo_config_index == 1 { 2 } else { asc_channels as u8 };
                        gc.flags_requested = 0;
                        gc.n_bands_analysis_requested = MAP_SBR_RATIO_2_N_ANA_BANDS
                            .get((sri as usize).wrapping_sub(1))
                            .copied()
                            .unwrap_or(0);
                        gc.n_bands_synthesis_requested = 64;
                        gc.n_qmf_time_slots_requested = if sri == 1 { 64 } else { 32 };
                        gc.n_qmf_ov_time_slots_requested = if sri == 1 { 12 } else { 6 };
                        gc.n_qmf_proc_bands_requested = 64;
                        gc.n_qmf_proc_channels_requested = 1;
                        gc.park_channel = (usac_stereo_config_index == 3) as u8;
                        gc.park_channel_requested = (usac_stereo_config_index == 3) as u8;
                        gc.qmf_domain_explicit_config = 1;
                    }
                }
                AOT_ER_AAC_ELD => {
                    if mps_enable_curr != 0
                        && asc.m_sc.m_eld_specific_config.m_use_ld_qmf_time_align != 0
                    {
                        let sac_interface = if sbr_enabled != 0 && self_.h_sbr_decoder.is_some() {
                            SAC_INTERFACE_QMF
                        } else {
                            SAC_INTERFACE_TIME
                        };
                        mpeg_surround_decoder_configure_qmf_domain(
                            self_.p_mpeg_surround_decoder.as_deref_mut(),
                            &mut self_.qmf_domain,
                            sac_interface,
                            self_.stream_info.aac_sample_rate as u32,
                            asc.m_aot,
                        );
                        self_.qmf_domain.global_conf.qmf_domain_explicit_config = 1;
                    }
                }
                _ => {
                    /* qmfDomain is initialized by SBR and MPS init functions if required */
                    self_.qmf_domain.global_conf.qmf_domain_explicit_config = 0;
                }
            }

            /* Allocate all memory structures for each channel */
            {
                let mut ch = aac_channels_offset;
                for _ch in 0..asc_channels {
                    if ch >= 8 {
                        break 'bail;
                    }
                    self_.p_aac_decoder_channel_info[ch] =
                        Some(get_aac_decoder_channel_info(ch as i32));
                    /* This is temporary until the DynamicData is split into two or more regions!
                    The memory could be reused after completed core decoding. */
                    ch += 1;
                }
                // C allocates new element memory; the old allocations are freed by
                // CAacDecoder_DeInit() (or leaked, and never accessed again: they belong to
                // channels at or above ascChannels).
                self_.com_data.clear();
                self_.com_static_data.clear();
                self_.p_com_data = [None; NCH];
                self_.p_com_static_data = [None; NCH];

                let mut ch_idx = aac_channels_offset_idx;
                ch = aac_channels_offset;
                let mut num_elements: i32 = 8 + 8;
                if flags & (AC_RSV603DA | AC_USAC) != 0 {
                    num_elements = asc.m_sc.m_usac_config.m_usac_num_elements as i32;
                }
                for el_ in 0..num_elements.max(0) as usize {
                    let el = element_offset + el_;
                    if el >= MAX_ELEMENTS {
                        break;
                    }

                    if flags & (AC_ER | AC_LD | AC_ELD | AC_RSV603DA | AC_USAC | AC_RSVD50) != 0
                        && ch as i32 >= asc_channels
                    {
                        break;
                    }

                    match self_.elements[el] {
                        ID_SCE | ID_CPE | ID_LFE | ID_USAC_SCE | ID_USAC_CPE | ID_USAC_LFE => {
                            let el_channels = caac_decoder_get_el_channels(
                                self_.elements[el],
                                self_.usac_stereo_config_index[el],
                            );

                            // pAacDecoderChannelInfo[ch] exists below ascChannels; beyond them
                            // the reference has no defined result, and the port bails out.
                            if ch >= NCH || self_.p_aac_decoder_channel_info[ch].is_none() {
                                break 'bail;
                            }
                            let (cs, cd) =
                                alloc_element_memory(self_, ch, ch == aac_channels_offset);

                            if el_channels == 2 {
                                if ch >= 8 - 1 {
                                    return AAC_DEC_UNSUPPORTED_CHANNELCONFIG;
                                }
                                if self_.p_aac_decoder_channel_info[ch + 1].is_none() {
                                    break 'bail;
                                }
                                self_.p_com_data[ch + 1] = Some(cd);
                                self_.p_com_static_data[ch + 1] = Some(cs);
                            }

                            ch += el_channels as usize;
                            ch_idx += el_channels;
                        }
                        _ => {}
                    }

                    if self_.elements[el] == ID_END {
                        break;
                    }

                    // `el++` at the end of the loop body has no effect (el is recomputed).
                }
                let _ = ch_idx;

                ch = aac_channels_offset;
                for _ch in 0..asc_channels {
                    /* Allocate persistent channel memory */
                    let mut sci = get_aac_decoder_static_channel_info(ch as i32);
                    sci.i_mdct.overlap = get_overlap_buffer(ch as i32); /* This area size depends on the AOT */
                    if flags & (AC_USAC | AC_RSVD50 | AC_RSV603DA/*|AC_BSAC*/) != 0 {
                        sci.h_ar_co = Some(carco_create());
                    }
                    self_.p_aac_decoder_static_channel_info[ch] = Some(sci);

                    if flags & (AC_USAC | AC_RSV603DA) == 0 {
                        // CPns_UpdateNoiseState(): randomSeed is not stored (see
                        // crate::channelinfo::aacdec_pns_types).
                        if let Some(ci) = self_.p_aac_decoder_channel_info[ch].as_deref_mut() {
                            cpns_update_noise_state(&mut ci.data.aac.pns_data, Some(ch));
                        }
                    }
                    ch += 1;
                }

                if flags & AC_USAC != 0 {
                    for ch_ in 0..flush_channels.max(0) as usize {
                        ch = aac_channels_offset + ch_;
                        if self_.p_time_data_flush[ch].is_none() {
                            self_.p_time_data_flush[ch] = Some(get_time_data_flush(ch as i32));
                        }
                    }
                }

                if flags & (AC_USAC | AC_RSV603DA) != 0 {
                    let mut complex_stereo_pred_possible = 0;
                    ch = aac_channels_offset;
                    let uc = &asc.m_sc.m_usac_config;
                    for el2_ in 0..uc.m_usac_num_elements.min(MAX_ELEMENTS as u32) as usize {
                        let el2 = element_offset + el2_;
                        let mut el_ch = 0usize;

                        if (self_.elements[el2] == ID_USAC_CPE)
                            && self_.usac_stereo_config_index[el2] != 1
                        {
                            el_ch = 2;
                        } else if is_channel_element(self_.elements[el2]) {
                            el_ch = 1;
                        }

                        if el_flags[el2] & AC_EL_USAC_CP_POSSIBLE != 0 {
                            complex_stereo_pred_possible = 1;
                            if self_.cpe_static_data[el2].is_none() {
                                self_.cpe_static_data[el2] = Some(get_cpe_persistent_data(0));
                            }
                        }

                        for _ch2 in 0..el_ch {
                            /* Hook element specific cpeStaticData into channel specific
                             * aacDecoderStaticChannelInfo */
                            let cpe =
                                if self_.cpe_static_data[el2].is_some() { Some(el2) } else { None };
                            if let Some(sci) = self_
                                .p_aac_decoder_static_channel_info
                                .get_mut(ch)
                                .and_then(|s| s.as_deref_mut())
                            {
                                sci.p_cpe_static_data = cpe;
                                // spectralCoeffs[ch2], specScale[ch2] and scratchBuffer are not
                                // stored (see crate::channelinfo::stereo_types).
                            }
                            ch += 1;
                        } /* for each channel in current element */
                        if complex_stereo_pred_possible != 0 && (el_ch == 2) {
                            /* needed once for all channels */
                            if let Some(cs) = self_.p_com_static_data.get(ch - 1).copied().flatten()
                            {
                                let csd = &mut self_.com_static_data[cs];
                                if csd.cplx_prediction_data.is_none() {
                                    csd.cplx_prediction_data = Some(get_cplx_prediction_data(0));
                                }
                            } else {
                                break 'bail;
                            }
                        }
                        if el_ch > 0 {
                            if let Some(sci) = self_
                                .p_aac_decoder_static_channel_info
                                .get_mut(ch - el_ch)
                                .and_then(|s| s.as_deref_mut())
                            {
                                sci.nf_random_seed = 0x3039;
                            }
                            if self_.elements[el2] == ID_USAC_CPE {
                                // C indexes element[el2] (not el2_; the same with elementOffset 0).
                                let sci_idx =
                                    uc.element.get(el2).map_or(0, |e| e.m_stereo_config_index);
                                if sci_idx != 1 {
                                    if let Some(sci) = self_
                                        .p_aac_decoder_static_channel_info
                                        .get_mut(ch - el_ch + 1)
                                        .and_then(|s| s.as_deref_mut())
                                    {
                                        sci.nf_random_seed = 0x10932;
                                    }
                                }
                            }
                        }
                    } /* for each element */
                }

                if asc_channels != self_.aac_channels {
                    /* Make allocated channel count persistent in decoder context. */
                    self_.aac_channels = (aac_channels_offset + ch) as i32;
                }
            }

            if usac_residual_delay_comp_samples != 0 {
                let delay_err = fdk_delay_create(
                    &mut self_.usac_residual_delay,
                    usac_residual_delay_comp_samples as u16,
                    1,
                );
                if delay_err != 0 {
                    break 'bail;
                }
            }

            /* Make amount of signalled channels persistent in decoder context. */
            self_.asc_channels[stream_index] = asc_channels;
            /* Init the previous channel count values. This is required to avoid a mismatch of
            memory accesses in the error concealment module and the allocated channel structures
            in this function. */
            self_.aac_channels_prev = 0;
        }

        if self_.p_aac_decoder_channel_info[0].is_some() {
            self_.p_drm_bs_buffer = true;
            self_.drm_bs_buffer_size = DRM_BS_BUFFER_SIZE as u16;
        }

        /* Update structures */
        if *config_changed != 0 {
            /* Things to be done for each channel, which do not involve allocating memory. Doing
            these things only on the channels needed for the current configuration
            (ascChannels) could lead to memory access violation later (error concealment). */
            let mut ch = 0usize;
            for _ch in 0..self_.asc_channels[stream_index] {
                if ch >= NCH {
                    break;
                }
                let (Some(ci), Some(sci)) = (
                    self_.p_aac_decoder_channel_info[ch].as_deref_mut(),
                    self_.p_aac_decoder_static_channel_info[ch].as_deref_mut(),
                ) else {
                    ch += 1;
                    continue;
                };
                match self_.stream_info.aot {
                    AOT_ER_AAC_ELD | AOT_ER_AAC_LD => {
                        ci.granule_length = self_.stream_info.aac_samples_per_frame;
                    }
                    _ => {
                        ci.granule_length = self_.stream_info.aac_samples_per_frame / 8;
                    }
                }
                ci.render_mode = init_render_mode;

                mdct_init(&mut sci.i_mdct, OverlapBufferSize);

                sci.last_core_mode = FD_LONG;
                sci.last_lpd_mode = 255;

                sci.last_tcx_pitch = L_DIV as i16;

                /* Reset DRC control data for this channel */
                aac_decoder_drc_init_channel_data(Some(&mut sci.drc_data));

                /* Delete mixdown metadata from the past */
                pcm_dmx_reset(self_.h_pcm_utils.as_deref_mut(), PCMDMX_RESET_BS_DATA);

                /* Reset concealment only if ASC changed. Otherwise it will be done with any
                config callback. E.g. every time the LATM SMC is present. */
                cconcealment_init_channel_data(
                    &mut sci.concealment_info,
                    init_render_mode,
                    self_.stream_info.aac_samples_per_frame,
                );
                ch += 1;
            }
        }

        if *config_changed != 0 {
            let (drc_dec_sample_rate, drc_dec_frame_size) =
                if self_.stream_info.ext_sampling_rate != 0 {
                    (
                        self_.stream_info.ext_sampling_rate,
                        (self_
                            .stream_info
                            .aac_samples_per_frame
                            .wrapping_mul(self_.stream_info.ext_sampling_rate))
                            / nz(self_.stream_info.aac_sample_rate),
                    )
                } else {
                    (self_.stream_info.aac_sample_rate, self_.stream_info.aac_samples_per_frame)
                };

            if fdk_drc_dec_init(
                self_.h_uni_drc_decoder.as_deref_mut(),
                drc_dec_frame_size,
                drc_dec_sample_rate,
                self_.aac_channels,
            ) != 0
            {
                break 'bail;
            }
        }

        if *config_changed != 0 && asc.m_aot == AOT_USAC {
            if let Some(d) = self_.h_drc_info.as_deref_mut() {
                aac_decoder_drc_disable(d);
            }
        }

        if asc.m_aot == AOT_USAC {
            pcm_limiter_set_attack(self_.h_limiter.as_deref_mut(), 5);
            pcm_limiter_set_threshold(
                self_.h_limiter.as_deref_mut(),
                fdk_aac_rs_fdk::common_fix::fl2fxconst_dbl(0.89125094f32 as f64),
            );
        }

        caac_decoder_accept_flags(self_, flags, &el_flags, stream_index);
        self_.sbr_enabled = sbr_enabled;
        self_.sbr_enabled_prev = sbr_enabled_prev;
        self_.mps_enable_curr = mps_enable_curr;

        /* Update externally visible copy of flags */
        self_.stream_info.flags = self_.flags[0];

        return err;
    }

    /* bail: */
    caac_decoder_de_init(self_, 0);
    AAC_DEC_OUT_OF_MEMORY
}

/// The present entries `start..start + n` of a per channel array (the channels of an element).
pub(crate) fn channels_mut<T>(
    arr: &mut [Option<Box<T>>],
    start: usize,
    n: usize,
) -> ChanRefs<'_, T> {
    let end = (start + n).min(arr.len());
    let start = start.min(end);
    let mut it = arr[start..end].iter_mut().filter_map(|c| c.as_deref_mut());
    // the elements have one or two channels: no heap allocation for them
    if end - start <= 2 {
        match (it.next(), it.next()) {
            (None, _) => ChanRefs::A0([]),
            (Some(a), None) => ChanRefs::A1([a]),
            (Some(a), Some(b)) => ChanRefs::A2([a, b]),
        }
    } else {
        ChanRefs::V(it.collect())
    }
}

/// The result of [`channels_mut`]: the references in a vector, or in an array for up to two.
pub(crate) enum ChanRefs<'a, T> {
    A0([&'a mut T; 0]),
    A1([&'a mut T; 1]),
    A2([&'a mut T; 2]),
    V(Vec<&'a mut T>),
}

impl<'a, T> core::ops::Deref for ChanRefs<'a, T> {
    type Target = [&'a mut T];
    fn deref(&self) -> &Self::Target {
        match self {
            ChanRefs::A0(a) => a,
            ChanRefs::A1(a) => a,
            ChanRefs::A2(a) => a,
            ChanRefs::V(v) => v,
        }
    }
}

impl<T> core::ops::DerefMut for ChanRefs<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        match self {
            ChanRefs::A0(a) => a,
            ChanRefs::A1(a) => a,
            ChanRefs::A2(a) => a,
            ChanRefs::V(v) => v,
        }
    }
}

/// `&buf[off..]`, empty if `off` is beyond the buffer.
#[inline]
pub(crate) fn tail_mut<T>(buf: &mut [T], off: usize) -> &mut [T] {
    let off = off.min(buf.len());
    &mut buf[off..]
}

/// `CAacDecoder_DecodeFrame(self, flags, pTimeData, timeDataSize, timeDataChannelOffset)`:
/// decodes one AAC frame. The decoding of coupling channel elements is not supported. The
/// transport layer might signal that the data of the current frame is invalid, e.g. as a result
/// of a packet loss in streaming mode. The bitstream position of the transport decoder must be
/// exactly the end of the access unit, including all byte alignment bits.
///
/// `p_time_data` is the decoder's `pTimeData2` (`timeDataSize` values; the joint stereo scratch
/// buffer as well).
pub fn caac_decoder_decode_frame(
    self_: &mut AacDecoderInstance,
    flags: u32,
    p_time_data: &mut [PcmDec],
    time_data_size: i32,
    time_data_channel_offset: i32,
) -> AacDecoderError {
    let Some(tp) = self_.h_input.as_deref_mut() else {
        // No transport decoder: the reference has no defined result here.
        return AAC_DEC_UNKNOWN;
    };
    let mut bs: FdkBitStream = tp.bit_stream[0].clone();
    let r = decode_frame_core(
        self_,
        flags,
        p_time_data,
        time_data_size,
        time_data_channel_offset,
        &mut bs,
    );
    if let Some(tp) = self_.h_input.as_deref_mut() {
        tp.bit_stream[0] = bs;
    }
    r
}

#[allow(clippy::cognitive_complexity)]
fn decode_frame_core(
    self_: &mut AacDecoderInstance,
    flags: u32,
    p_time_data: &mut [PcmDec],
    time_data_size: i32,
    time_data_channel_offset: i32,
    bs: &mut FdkBitStream,
) -> AacDecoderError {
    let mut error_status = AAC_DEC_OK;

    let mut type_: Mp4ElementId = ID_NONE; /* Current element type */
    let mut aac_channels: i32 = 0; /* Channel counter for channels found in the bitstream */
    let stream_index: usize = 0; /* index of the current substream */

    let au_start_anchor = fdk_get_valid_bits(bs) as i32; /* AU start bit buffer position for AU byte alignment */

    let mut check_sample_rate = self_.stream_info.aac_sample_rate;

    let mut cconceal_td_fading_applied = [0i32; NCH]; /* Initialize status of Time Domain fading */

    if self_.aac_channels <= 0 {
        return AAC_DEC_UNSUPPORTED_CHANNELCONFIG;
    }

    /* Any supported base layer valid AU will require more than 16 bits. */
    if (self_.with_tp(bs, |tp| transport_dec_get_au_bits_remaining(tp, 0)) < 15)
        && (flags & (AACDEC_CONCEAL | AACDEC_FLUSH)) == 0
    {
        self_.frame_ok = 0;
        error_status = AAC_DEC_DECODE_FRAME_ERROR;
    }

    /* Reset Program Config structure */
    cprogram_config_reset(&mut self_.pce);

    caac_decoder_anc_data_reset(&mut self_.anc_data);
    if (flags & (AACDEC_CONCEAL | AACDEC_FLUSH)) == 0
        && (self_.flags[0] & (AC_USAC | AC_RSV603DA)) == 0
        && self_.stream_info.channel_config == 0
    {
        /* Init Channel/Element mapping table */
        for ch in 0..8 {
            self_.ch_mapping[ch] = 255;
        }
        if cprogram_config_is_valid(&self_.pce) == 0 {
            for el in 0..MAX_ELEMENTS {
                self_.elements[el] = ID_NONE;
            }
        }
    }

    if self_.downscale_factor > 1 && (self_.flags[0] & AC_ELD) != 0 {
        self_.flags[0] |= AC_ELD_DOWNSCALE;
    } else {
        self_.flags[0] &= !AC_ELD_DOWNSCALE;
    }
    /* unsupported dsf (aacSampleRate has not yet been divided by dsf) -> divide */
    if self_.downscale_factor_in_bs > 1 && (self_.flags[0] & AC_ELD_DOWNSCALE) == 0 {
        check_sample_rate = self_.stream_info.aac_sample_rate / self_.downscale_factor_in_bs as i32;
    }

    /* Check sampling frequency  */
    if self_.stream_info.aac_sample_rate <= 0 {
        /* Instance maybe uninitialized! */
        return AAC_DEC_UNSUPPORTED_SAMPLINGRATE;
    }
    match check_sample_rate {
        96000 | 88200 | 64000 | 16000 | 12000 | 11025 | 8000 | 7350 | 48000 | 44100 | 32000
        | 24000 | 22050 => {}
        _ => {
            if self_.flags[0] & (AC_USAC | AC_RSVD50 | AC_RSV603DA) == 0 {
                return AAC_DEC_UNSUPPORTED_SAMPLINGRATE;
            }
        }
    }

    if flags & AACDEC_CLRHIST != 0 && self_.flags[0] & AC_USAC == 0 {
        /* Clear history */
        let render_mode = self_.p_aac_decoder_channel_info[0]
            .as_deref()
            .map_or(AACDEC_RENDER_INVALID, |c| c.render_mode);
        for ch in 0..self_.aac_channels.clamp(0, NCH as i32) as usize {
            if let Some(sci) = self_.p_aac_decoder_static_channel_info[ch].as_deref_mut() {
                /* Reset concealment */
                cconcealment_init_channel_data(
                    &mut sci.concealment_info,
                    render_mode,
                    self_.stream_info.aac_samples_per_frame,
                );
                /* Clear overlap-add buffers to avoid clicks. */
                let n = (OverlapBufferSize as usize).min(sci.i_mdct.overlap.len());
                sci.i_mdct.overlap[..n].fill(0);
            }
        }
        if self_.stream_info.channel_config > 0 {
            /* Declare the possibly adopted old PCE (with outdated metadata) invalid. */
            cprogram_config_init(&mut self_.pce);
        }
    }

    let mut pce_read = 0; /* Flag indicating a PCE in the current raw_data_block() */

    let hdaac_decoded: i32 = 0;
    let mut previous_element: Mp4ElementId = ID_END; /* Last element ID (required for extension payload mapping */
    let mut previous_element_index: u8 = 0; /* Canonical index of last element */
    let mut element_count: i32 = 0; /* Element counter for elements found in the bitstream */
    let mut channel_element_count: i32 = 0; /* Channel element counter */
    /* Channel elements in bit stream order. */
    let mut channel_elements = [ID_NONE; MAX_ELEMENTS];
    let mut el_cnt = [0i32; ID_LAST as usize]; /* element counter ( robustness ) */
    let element_count_prev_streams: i32 = 0; /* Element count of all previous sub streams. */

    while (type_ != ID_END) && (flags & (AACDEC_CONCEAL | AACDEC_FLUSH)) == 0 && self_.frame_ok != 0
    {
        let el_channels: i32;

        if self_.flags[0] & (AC_USAC | AC_RSVD50 | AC_RSV603DA | AC_ELD | AC_SCALABLE | AC_ER) == 0
        {
            type_ = fdk_read_bits(bs, 3) as Mp4ElementId;
        } else {
            if element_count as usize >= MAX_ELEMENTS {
                self_.frame_ok = 0;
                error_status = AAC_DEC_PARSE_ERROR;
                break;
            }
            type_ = self_.elements[element_count as usize];
        }

        if ((self_.flags[stream_index] & (AC_USAC | AC_RSVD50)) != 0 && element_count == 0)
            || (self_.flags[stream_index] & AC_RSV603DA) != 0
        {
            self_.flags[stream_index] &= !AC_INDEP;

            if fdk_read_bit(bs) != 0 {
                self_.flags[stream_index] |= AC_INDEP;
            }

            let mut ch = aac_channels;
            let mut ch_idx = aac_channels;
            while ch_idx < self_.asc_channels[stream_index] {
                {
                    /* Robustness check */
                    if ch >= self_.aac_channels {
                        return AAC_DEC_UNKNOWN;
                    }

                    /* if last frame was broken and this frame is no independent frame, correct
                     * decoding is impossible we need to trigger concealment */
                    let last_ok = cconcealment_get_last_frame_ok(
                        self_
                            .p_aac_decoder_static_channel_info
                            .get(ch as usize)
                            .and_then(|s| s.as_deref())
                            .map(|s| &s.concealment_info),
                        1,
                    );
                    if last_ok == 0 && (self_.flags[stream_index] & AC_INDEP) == 0 {
                        self_.frame_ok = 0;
                    }
                    ch += 1;
                }
                ch_idx += 1;
            }
        }

        if (fdk_get_valid_bits(bs) as i32) < 0 {
            self_.frame_ok = 0;
        }

        match type_ {
            ID_SCE | ID_CPE | ID_LFE | ID_USAC_SCE | ID_USAC_CPE | ID_USAC_LFE => 'el: {
                if element_count as usize >= MAX_ELEMENTS {
                    self_.frame_ok = 0;
                    error_status = AAC_DEC_PARSE_ERROR;
                    break 'el;
                }

                el_channels = caac_decoder_get_el_channels(
                    type_,
                    self_.usac_stereo_config_index[element_count as usize],
                );

                /*
                 Consistency check
                */
                {
                    let mut total_asc_channels = 0;

                    for i in 0..1 {
                        total_asc_channels += self_.asc_channels[i];
                    }
                    if (el_cnt[type_ as usize] >= (total_asc_channels >> (el_channels - 1)))
                        || (aac_channels > (total_asc_channels - el_channels))
                    {
                        error_status = AAC_DEC_DECODE_FRAME_ERROR;
                        self_.frame_ok = 0;
                        break 'el;
                    }
                }

                let a = aac_channels as usize;

                if self_.flags[stream_index] & (AC_USAC | AC_RSVD50 | AC_RSV603DA) == 0 {
                    for ch in 0..el_channels as usize {
                        let com = self_.p_com_data.get(a + ch).copied().flatten();
                        if let (Some(ci), Some(cd)) = (
                            self_
                                .p_aac_decoder_channel_info
                                .get_mut(a + ch)
                                .and_then(|c| c.as_deref_mut()),
                            com,
                        ) {
                            cpns_reset_data(
                                &mut ci.data.aac.pns_data,
                                &mut self_.com_data[cd].pns_inter_channel_data,
                            );
                        }
                    }
                }

                if self_.frame_ok != 0 {
                    error_status =
                        read_channel_element(self_, bs, a, el_channels, element_count as usize);
                    if error_status != AAC_DEC_OK {
                        self_.frame_ok = 0;
                    }
                }

                if self_.frame_ok != 0 {
                    /* Lookup the element and decode it only if it belongs to the current program */
                    let tag = self_.p_aac_decoder_channel_info[a.min(NCH - 1)]
                        .as_deref()
                        .map_or(0, |c| c.element_instance_tag);
                    if cprogram_config_lookup_element(
                        &mut self_.pce,
                        self_.stream_info.channel_config as u32,
                        tag as u32,
                        aac_channels as u32,
                        &mut self_.ch_mapping,
                        &mut self_.stream_info.p_channel_type,
                        &mut self_.stream_info.p_channel_indices,
                        8,
                        &mut previous_element_index,
                        &mut self_.elements,
                        type_,
                    ) != 0
                    {
                        channel_elements[channel_element_count as usize] = type_;
                        channel_element_count += 1;
                        aac_channels += el_channels;
                    } else {
                        self_.frame_ok = 0;
                    }
                    /* Create SBR element for SBR for upsampling for LFE elements, and if SBR was
                    implicitly signaled, because the first frame(s) may not contain SBR payload
                    (broken encoder, bit errors). */
                    if self_.frame_ok != 0
                        && ((self_.flags[stream_index] & AC_SBR_PRESENT) != 0
                            || (self_.sbr_enabled == 1))
                        && (self_.flags[stream_index] & AC_USAC) == 0
                    /* Is done during explicit config set up */
                    {
                        let mut config_mode: u8 = 0;
                        let mut config_changed: u8 = 0;
                        config_mode |= AC_CM_ALLOC_MEM;

                        let sbr_error = sbr_init_element(
                            self_.h_sbr_decoder.as_deref_mut(),
                            &mut self_.qmf_domain,
                            self_.stream_info.aac_sample_rate,
                            self_.stream_info.ext_sampling_rate,
                            self_.stream_info.aac_samples_per_frame,
                            self_.stream_info.aot,
                            type_,
                            previous_element_index as i32,
                            2, /* Signalize that harmonicSBR shall be ignored in the config
                               change detection */
                            0,
                            config_mode,
                            &mut config_changed,
                            self_.downscale_factor as i32,
                        );
                        if sbr_error != SBRDEC_OK {
                            /* Do not try to apply SBR because initializing the element failed. */
                            self_.sbr_enabled = 0;
                        }
                    }
                }

                el_cnt[type_ as usize] += 1;
                if self_.frame_ok != 0
                    && (self_.flags[stream_index] & AC_USAC) != 0
                    && (type_ == ID_USAC_CPE || type_ == ID_USAC_SCE)
                {
                    error_status = aac_decoder_parse_explicit_mps_and_sbr(
                        self_,
                        bs,
                        previous_element,
                        previous_element_index as i32,
                        element_count,
                        &el_cnt,
                    );
                    if error_status != AAC_DEC_OK {
                        self_.frame_ok = 0;
                    }
                }
            }

            ID_CCE => 'el: {
                /*
                  Consistency check
                */
                if el_cnt[type_ as usize] > self_.asc_channels[stream_index] {
                    error_status = AAC_DEC_DECODE_FRAME_ERROR;
                    self_.frame_ok = 0;
                    break 'el;
                }

                if self_.frame_ok != 0 {
                    error_status =
                        read_coupling_channel_element(self_, bs, &mut previous_element_index);
                }
                el_cnt[type_ as usize] += 1;
            }

            ID_DSE => {
                let mut element_instance_tag: u8 = 0;

                cdata_stream_element_read(
                    self_,
                    bs,
                    &mut element_instance_tag,
                    au_start_anchor as u32,
                );

                if cprogram_config_lookup_element(
                    &mut self_.pce,
                    self_.stream_info.channel_config as u32,
                    element_instance_tag as u32,
                    0,
                    &mut self_.ch_mapping,
                    &mut self_.stream_info.p_channel_type,
                    &mut self_.stream_info.p_channel_indices,
                    8,
                    &mut previous_element_index,
                    &mut self_.elements,
                    type_,
                ) == 0
                {
                    /* most likely an error in bitstream occured */
                    // self->frameOK = 0;
                }
            }

            ID_PCE => {
                let channel_config = self_.stream_info.channel_config as u32;
                let result = cprogram_config_element_read(
                    bs,
                    self_.h_input.as_deref_mut(),
                    &mut self_.pce,
                    channel_config,
                    au_start_anchor as u32,
                );
                if result < 0 {
                    /* Something went wrong */
                    error_status = AAC_DEC_PARSE_ERROR;
                    self_.frame_ok = 0;
                } else if result > 1 {
                    /* Built element table */
                    let mut el_idx = cprogram_config_get_element_table(
                        &self_.pce,
                        &mut self_.elements,
                        (8 + 8) as i32,
                        &mut self_.ch_map_index,
                    );
                    /* Reset the remaining tabs */
                    while (el_idx as usize) < MAX_ELEMENTS {
                        self_.elements[el_idx.max(0) as usize] = ID_NONE;
                        el_idx += 1;
                    }
                    /* Make new number of channel persistent */
                    self_.asc_channels[stream_index] = self_.pce.num_channels as i32;
                    /* If PCE is not first element conceal this frame to avoid inconsistencies */
                    if element_count != 0 {
                        self_.frame_ok = 0;
                    }
                }
                pce_read = if result >= 0 { 1 } else { 0 };
            }

            ID_FIL => {
                let mut bit_cnt = fdk_read_bits(bs, 4) as i32; /* bs_count */

                if bit_cnt == 15 {
                    let esc_count = fdk_read_bits(bs, 8) as i32; /* bs_esc_count */
                    bit_cnt = esc_count + 14;
                }

                /* Convert to bits */
                bit_cnt <<= 3;

                while bit_cnt > 0 {
                    error_status = caac_decoder_ext_payload_parse(
                        self_,
                        bs,
                        &mut bit_cnt,
                        previous_element,
                        previous_element_index as i32,
                        1,
                    );
                    if error_status != AAC_DEC_OK {
                        self_.frame_ok = 0;
                        break;
                    }
                }
            }

            ID_EXT => 'el: {
                if element_count as usize >= MAX_ELEMENTS {
                    self_.frame_ok = 0;
                    error_status = AAC_DEC_PARSE_ERROR;
                    break 'el;
                }

                error_status = aac_decoder_parse_explicit_mps_and_sbr(
                    self_,
                    bs,
                    previous_element,
                    previous_element_index as i32,
                    element_count,
                    &el_cnt,
                );
            }

            ID_USAC_EXT => 'el: {
                if (element_count - element_count_prev_streams) >= TP_USAC_MAX_ELEMENTS as i32 {
                    self_.frame_ok = 0;
                    error_status = AAC_DEC_PARSE_ERROR;
                    break 'el;
                }
                /* parse extension element payload q.v. rsv603daExtElement() ISO/IEC DIS
                23008-3  Table 30 or   UsacExElement() ISO/IEC FDIS 23003-3:2011(E)  Table 21
                 */
                let ext = self_
                    .usac_config()
                    .and_then(|uc| {
                        uc.element.get((element_count - element_count_prev_streams) as usize)
                    })
                    .map(|e| e.ext_element)
                    .unwrap_or_default();

                if fdk_read_bit(bs) != 0 {
                    /* usacExtElementPresent */
                    let mut usac_ext_element_payload_length: i32;
                    if fdk_read_bit(bs) != 0 {
                        /* usacExtElementUseDefaultLength */
                        usac_ext_element_payload_length =
                            ext.usac_ext_element_default_length as i32;
                    } else {
                        usac_ext_element_payload_length = fdk_read_bits(bs, 8) as i32;
                        if usac_ext_element_payload_length as u32 == (1u32 << 8) - 1 {
                            let value_add = fdk_read_bits(bs, 16);
                            usac_ext_element_payload_length =
                                usac_ext_element_payload_length.wrapping_add(value_add as i32 - 2);
                        }
                    }
                    if usac_ext_element_payload_length > 0 {
                        if ext.usac_ext_element_payload_frag != 0 {
                            /* usacExtElementStart = */
                            fdk_read_bit(bs);
                            /* usacExtElementStop = */
                            fdk_read_bit(bs);
                        } else {
                            /* usacExtElementStart = 1; */
                            /* usacExtElementStop = 1; */
                        }

                        let mut usac_ext_bit_pos = fdk_get_valid_bits(bs) as i32;

                        let usac_ext_element_type = ext.usac_ext_element_type;

                        if usac_ext_element_type == ID_EXT_ELE_UNI_DRC {
                            /* uniDrcGain() */
                            if stream_index == 0 {
                                let drc_err = fdk_drc_dec_read_uni_drc_gain(
                                    self_.h_uni_drc_decoder.as_deref_mut(),
                                    bs,
                                );
                                if drc_err != 0 {
                                    error_status = AAC_DEC_PARSE_ERROR;
                                }
                            }
                        }

                        /* Skip any remaining bits of extension payload */
                        usac_ext_bit_pos = (usac_ext_element_payload_length.wrapping_mul(8))
                            .wrapping_sub(
                                usac_ext_bit_pos.wrapping_sub(fdk_get_valid_bits(bs) as i32),
                            );
                        if usac_ext_bit_pos < 0 {
                            self_.frame_ok = 0;
                            error_status = AAC_DEC_PARSE_ERROR;
                        }
                        fdk_push_bi_directional(bs, usac_ext_bit_pos);
                    }
                }
            }

            ID_END | ID_USAC_END => {}

            _ => {
                error_status = AAC_DEC_DECODE_FRAME_ERROR;
                self_.frame_ok = 0;
            }
        }

        previous_element = type_;
        element_count += 1;
    } /* while ( (type != ID_END) ... ) */

    if (self_.flags[stream_index]
        & (AC_USAC | AC_RSVD50 | AC_RSV603DA | AC_BSAC | AC_LD | AC_ELD | AC_ER | AC_SCALABLE))
        == 0
        && (self_.stream_info.channel_config == 0)
        && self_.pce.is_valid != 0
        && (error_status == AAC_DEC_OK)
        && self_.frame_ok != 0
        && (flags & (AACDEC_CONCEAL | AACDEC_FLUSH)) == 0
    {
        /* Check whether all PCE listed element instance tags are present in raw_data_block() */
        if validate_element_instance_tags(
            &self_.pce,
            &self_.p_aac_decoder_channel_info,
            aac_channels,
            &channel_elements,
            channel_element_count.min(MAX_ELEMENTS as i32),
        ) == 0
        {
            error_status = AAC_DEC_DECODE_FRAME_ERROR;
            self_.frame_ok = 0;
        }
    }

    if flags & (AACDEC_CONCEAL | AACDEC_FLUSH) == 0 {
        let preroll_len =
            self_.preroll_au_length.get(self_.access_unit as usize).copied().unwrap_or(0);
        /* float decoder checks if bitsLeft is in range 0-7; only prerollAUs are byteAligned with
         * respect to the first bit */
        /* Byte alignment with respect to the first bit of the raw_data_block(). */
        if (self_.flags[stream_index] & (AC_RSVD50 | AC_USAC)) == 0 || preroll_len != 0
        /* indicates preroll */
        {
            fdk_byte_align(bs, au_start_anchor as u32);
        }

        /* Check if all bits of the raw_data_block() have been read. */
        if self_.with_tp(bs, |tp| transport_dec_get_au_bits_total(tp, 0)) > 0 {
            let mut unread_bits =
                self_.with_tp(bs, |tp| transport_dec_get_au_bits_remaining(tp, 0));
            /* for pre-roll frames pre-roll length has to be used instead of total AU lenght */
            /* unreadBits regarding preroll bounds */
            if preroll_len != 0 {
                unread_bits = unread_bits
                    .wrapping_sub(self_.with_tp(bs, |tp| transport_dec_get_au_bits_total(tp, 0)))
                    .wrapping_add(preroll_len as i32);
            }
            let usac = (self_.flags[stream_index] & (AC_RSVD50 | AC_USAC)) != 0;
            if (usac && ((unread_bits < 0) || (unread_bits > 7)) && preroll_len == 0)
                || ((!usac || preroll_len != 0) && (unread_bits != 0))
            {
                let is_drm = self_
                    .h_input
                    .as_deref()
                    .is_some_and(|tp| transport_dec_get_format(tp) == TT_DRM);
                if (((unread_bits < 0) || (unread_bits > 7)) && self_.frame_ok != 0)
                    && (is_drm && (self_.flags[stream_index] & AC_USAC) != 0)
                {
                    /* Set frame OK because of fill bits. */
                    self_.frame_ok = 1;
                } else {
                    self_.frame_ok = 0;
                }

                /* Do not overwrite current error */
                if error_status == AAC_DEC_OK && self_.frame_ok == 0 {
                    error_status = AAC_DEC_PARSE_ERROR;
                }
                /* Always put the bitbuffer at the right position after the current Access
                 * Unit. */
                fdk_push_bi_directional(bs, unread_bits);
            }
        }

        /* Check the last element. The terminator (ID_END) has to be the last one (even if ER
         * syntax is used). */
        if self_.frame_ok != 0 && type_ != ID_END {
            /* Do not overwrite current error */
            if error_status == AAC_DEC_OK {
                error_status = AAC_DEC_PARSE_ERROR;
            }
            self_.frame_ok = 0;
        }
    }

    if (flags & (AACDEC_CONCEAL | AACDEC_FLUSH)) == 0 && self_.frame_ok != 0 {
        // channel_element_count is at most 8 here (the consistency check of the channel
        // elements); a write outside the array is dropped.
        if let Some(e) = channel_elements.get_mut(channel_element_count as usize) {
            *e = ID_END;
        }
        channel_element_count += 1;
    }
    let _ = channel_element_count;
    element_count = 0;
    aac_channels = 0;
    type_ = ID_NONE;
    previous_element_index = 0;
    let _ = previous_element_index;

    while type_ != ID_END && (element_count as usize) < MAX_ELEMENTS {
        let ec = element_count as usize;

        if (flags & (AACDEC_CONCEAL | AACDEC_FLUSH)) != 0 || self_.frame_ok == 0 {
            channel_elements[ec] = self_.elements[ec];
            if channel_elements[ec] == ID_NONE {
                channel_elements[ec] = ID_END;
            }
        }

        if self_.flags[stream_index] & (AC_USAC | AC_RSV603DA | AC_BSAC) != 0 {
            type_ = self_.elements[ec];
        } else {
            type_ = channel_elements[ec];
        }

        if (flags & (AACDEC_CONCEAL | AACDEC_FLUSH)) == 0 && self_.frame_ok != 0 {
            match type_ {
                ID_SCE | ID_CPE | ID_LFE | ID_USAC_SCE | ID_USAC_CPE | ID_USAC_LFE => {
                    let el_channels =
                        caac_decoder_get_el_channels(type_, self_.usac_stereo_config_index[ec]);

                    if hdaac_decoded == 0 {
                        // pCpeStaticData->jointStereoPersistentData.scratchBuffer = pTimeData:
                        // the scratch buffer is passed explicitly.
                        decode_channel_element(
                            self_,
                            aac_channels as usize,
                            el_channels,
                            ec,
                            p_time_data,
                        );
                    }
                    aac_channels += el_channels;
                }
                ID_NONE => {
                    type_ = ID_END;
                }
                _ => {}
            }
        }
        element_count += 1;
    }

    /* More AAC channels than specified by the ASC not allowed. */
    if (aac_channels == 0 || aac_channels > self_.aac_channels)
        && (flags & (AACDEC_CONCEAL | AACDEC_FLUSH)) == 0
    {
        /* Do not overwrite current error */
        if error_status == AAC_DEC_OK {
            error_status = AAC_DEC_DECODE_FRAME_ERROR;
        }
        self_.frame_ok = 0;
        aac_channels = 0;
    }

    if (flags & (AACDEC_CONCEAL | AACDEC_FLUSH)) == 0
        && TRANSPORTDEC_OK != self_.with_tp(bs, transport_dec_crc_check)
    {
        error_status = AAC_DEC_CRC_ERROR;
        self_.frame_ok = 0;
    }

    /* Ensure that in case of concealment a proper error status is set. */
    if (self_.frame_ok == 0) && (error_status == AAC_DEC_OK) {
        error_status = AAC_DEC_DECODE_FRAME_ERROR;
    }

    if self_.frame_ok != 0 && (flags & AACDEC_FLUSH) != 0 {
        aac_channels = self_.aac_channels_prev;
        /* Because the downmix could be active, its necessary to restore the channel type and
         * indices. */
        self_.stream_info.p_channel_type = self_.channel_type_prev; /* restore */
        self_.stream_info.p_channel_indices = self_.channel_indices_prev; /* restore */
        self_.sbr_enabled = self_.sbr_enabled_prev;
    } else {
        /* store or restore the number of channels and the corresponding info */
        if self_.frame_ok != 0 && (flags & AACDEC_CONCEAL) == 0 {
            self_.aac_channels_prev = aac_channels; /* store */
            self_.channel_type_prev = self_.stream_info.p_channel_type; /* store */
            self_.channel_indices_prev = self_.stream_info.p_channel_indices; /* store */
            self_.sbr_enabled_prev = self_.sbr_enabled;
        } else if self_.aac_channels > 0 {
            if (self_.build_up_status == AACDEC_RSV60_BUILD_UP_ON)
                || (self_.build_up_status == AACDEC_RSV60_BUILD_UP_ON_IN_BAND)
                || (self_.build_up_status == AACDEC_USAC_BUILD_UP_ON)
            {
                aac_channels = self_.aac_channels;
                self_.aac_channels_prev = aac_channels; /* store */
            } else {
                aac_channels = self_.aac_channels_prev; /* restore */
            }
            self_.stream_info.p_channel_type = self_.channel_type_prev; /* restore */
            self_.stream_info.p_channel_indices = self_.channel_indices_prev; /* restore */
            self_.sbr_enabled = self_.sbr_enabled_prev;
        }
    }

    /* Update number of output channels */
    self_.stream_info.aac_num_channels = aac_channels;

    /* Ensure consistency of IS_OUTPUT_VALID() macro. */
    if aac_channels == 0 {
        error_status = AAC_DEC_UNKNOWN;
    }

    if pce_read == 1 && cprogram_config_is_valid(&self_.pce) != 0 {
        /* Set matrix mixdown infos if available from PCE. */
        pcm_dmx_set_matrix_mixdown_from_pce(
            self_.h_pcm_utils.as_deref_mut(),
            self_.pce.matrix_mixdown_index_present as i32,
            self_.pce.matrix_mixdown_index as i32,
            self_.pce.pseudo_surround_enable as i32,
        );
    }

    /* If there is no valid data to transfrom into time domain, return. */
    if is_output_valid(error_status) == 0 {
        return error_status;
    }

    /* Setup the output channel mapping. The table below shows the three possibilities:
     *   # | chCfg | PCE | chMapIndex
     *  ---+-------+-----+------------------
     *   1 |  > 0  |  no | chCfg
     *   2 |   0   | yes | cChCfg
     *   3 |   0   |  no | aacChannels || 0
     *  ---+-------+-----+--------+------------------
     * Where chCfg is the channel configuration index from ASC and cChCfg is a corresponding
     * chCfg derived from a given PCE. The variable aacChannels represents the number of channel
     * found during bitstream decoding. Due to the structure of the mapping table it can only be
     * used for mapping if its value is smaller than 7. Otherwise we use the fallback (0) which
     * is a simple pass-through. The possibility #3 should appear only with MPEG-2 (ADTS)
     * streams. This is mode is called "implicit channel mapping".
     */
    if (self_.stream_info.channel_config == 0) && self_.pce.is_valid == 0 {
        self_.ch_map_index = if aac_channels < 7 { aac_channels as u8 } else { 0 };
    }

    /*
      Inverse transform
    */
    {
        let mut f_copy_ch_map = 1;
        let mut drc_ch_map = [0u8; 8];

        if (self_.stream_info.channel_config == 0) && cprogram_config_is_valid(&self_.pce) != 0 {
            /* ISO/IEC 14496-3 says: If a PCE is present, the exclude_mask bits correspond to the
            audio channels in the SCE, CPE, CCE and LFE syntax elements in the order of their
            appearance in the PCE. In the case of a CPE, the first transmitted mask bit
            corresponds to the first channel in the CPE, the second transmitted mask bit to the
            second channel. In the case of a CCE, a mask bit is transmitted only if the coupling
            channel is specified to be an independently switched coupling channel. Thus we have
            to convert the internal channel mapping from "canonical" MPEG to PCE order: */
            let mut tmp_ch_map = [0u8; 8];
            if cprogram_config_get_pce_ch_map(&self_.pce, &mut tmp_ch_map, 8) == 0 {
                for c in 0..aac_channels.clamp(0, 8) as usize {
                    drc_ch_map[c] = if self_.ch_mapping[c] == 255 {
                        255
                    } else {
                        // chMapping values are channel indices below 8; other values read 0.
                        tmp_ch_map.get(self_.ch_mapping[c] as usize).copied().unwrap_or(0)
                    };
                }
                f_copy_ch_map = 0;
            }
        }
        if f_copy_ch_map != 0 {
            drc_ch_map.copy_from_slice(&self_.ch_mapping[..8]);
        }

        /* Extract DRC control data and map it to channels (without bitstream delay) */
        let mapped = aac_decoder_drc_prolog(
            self_.h_drc_info.as_deref_mut(),
            bs,
            &mut self_.p_aac_decoder_static_channel_info,
            self_.pce.element_instance_tag,
            &drc_ch_map,
            aac_channels,
        );
        if mapped > 0 {
            if self_.flags[stream_index] & (AC_USAC | AC_RSV603DA) == 0 {
                /* If at least one DRC thread has been mapped to a channel there was DRC data in
                 * the bitstream. */
                self_.flags[stream_index] |= AC_DRC_PRESENT;
            } else {
                error_status = AAC_DEC_UNSUPPORTED_FORMAT;
            }
        }
        if self_.flags[stream_index] & (AC_USAC | AC_RSV603DA) != 0 {
            if let Some(d) = self_.h_drc_info.as_deref_mut() {
                aac_decoder_drc_disable(d);
            }
        }

        /* Create a reverse mapping table */
        let mut reverse_ch_mapping = [0u8; 8 * 2];
        for c in 0..aac_channels.clamp(0, 16) {
            let mut d: i32 = 0;
            while d < aac_channels - 1 {
                if self_.ch_mapping[d as usize] as i32 == c {
                    break;
                }
                d += 1;
            }
            reverse_ch_mapping[c as usize] = d as u8;
        }

        // Not in C: the channel loop with the inverse transforms on several threads (a decoder
        // with more than one thread; none: the loop below).
        let par = crate::pipeline::chan::render_channels_pool(
            self_,
            crate::pipeline::inline_cap(self_),
            flags,
            element_count,
            &channel_elements,
            aac_channels,
            &reverse_ch_mapping,
            p_time_data,
            time_data_size,
            time_data_channel_offset,
            &mut cconceal_td_fading_applied,
            error_status,
        );
        if let Some(e) = par {
            error_status = e;
        }
        let seq_elements = if par.is_some() { 0 } else { element_count.max(0) as usize };

        let mut c: i32 = 0;
        let mut c_idx: i32 = 0;
        let mut el_channels_acc: i32 = 0;
        for el in 0..seq_elements {
            /* Force frame concealment during mute release active state. */
            let mut frame_ok_but_conceal = 0;

            if self_.flags[stream_index] & (AC_USAC | AC_RSV603DA | AC_BSAC) != 0 {
                type_ = self_.elements[el];
            } else {
                type_ = channel_elements[el];
            }

            {
                let n_element_channels =
                    caac_decoder_get_el_channels(type_, self_.usac_stereo_config_index[el]);

                el_channels_acc += n_element_channels;

                if n_element_channels == 0 {
                    continue;
                }
            }

            let mut el_ch = 0;
            /* "c" iterates in canonical MPEG channel order */
            while c_idx < el_channels_acc {
                /* Robustness check */
                if c >= aac_channels {
                    return AAC_DEC_UNKNOWN;
                }

                let cu = c as usize;
                let rev_c = reverse_ch_mapping[cu] as usize;

                /* Setup offset for time buffer traversal. */
                let offset: i32 = fdk_ch_map_descr_get_map_value(
                    &self_.map_descr,
                    reverse_ch_mapping[c_idx as usize],
                    self_.ch_map_index as u32,
                ) as i32
                    * time_data_channel_offset;

                let spec_off = cu * 1024;

                if flags & AACDEC_FLUSH != 0 {
                    /* Clear pAacDecoderChannelInfo->pSpectralCoefficient because with
                     * AACDEC_FLUSH set it contains undefined data. */
                    if let Some(wb2) = self_.work_buffer_core2.as_deref_mut() {
                        let s = tail_mut(wb2, spec_off);
                        let n =
                            (self_.stream_info.aac_samples_per_frame.max(0) as usize).min(s.len());
                        s[..n].fill(0);
                    }
                }

                let (Some(ci), Some(sci)) = (
                    self_.p_aac_decoder_channel_info[cu].as_deref_mut(),
                    self_
                        .p_aac_decoder_static_channel_info
                        .get_mut(rev_c)
                        .and_then(|s| s.as_deref_mut()),
                ) else {
                    // The channel infos below aacChannels exist (C dereferences them).
                    c += 1;
                    c_idx += 1;
                    el_ch += 1;
                    continue;
                };

                /* if The ics info is not valid and it will be stored and used in the following
                 * concealment method, mark the frame as erroneous */
                {
                    let h_concealment_info = &sci.concealment_info;
                    let mute_release_active = (self_.frame_ok != 0
                        && (flags & AACDEC_CONCEAL) == 0)
                        && ((h_concealment_info.conceal_state >= ConcealState_Mute)
                            && (h_concealment_info.cnt_valid_frames + 1
                                <= self_.conceal_common_data.num_mute_release_frames));
                    let ics_is_invalid = get_scale_factor_bands_transmitted(&ci.ics_info)
                        > get_scale_factor_bands_total(&ci.ics_info);
                    let ics_info_usedin_fade_out =
                        !(ci.render_mode == AACDEC_RENDER_LPD && sci.last_lpd_mode == 0);
                    if ics_info_usedin_fade_out && ics_is_invalid && !mute_release_active {
                        self_.frame_ok = 0;
                    }
                }

                let mut empty: [FixpDbl; 0] = [];
                let spec: &mut [FixpDbl] = match self_.work_buffer_core2.as_deref_mut() {
                    Some(wb2) => tail_mut(wb2, spec_off),
                    None => &mut empty,
                };

                /*
                  Conceal defective spectral data
                */
                let conceal_apply_return_code = {
                    let last_lpd_mode = sci.last_lpd_mode;
                    cconcealment_apply(
                        ci,
                        sci,
                        &self_.sampling_rate_info[stream_index],
                        self_.stream_info.aac_samples_per_frame,
                        last_lpd_mode,
                        (self_.frame_ok != 0 && (flags & AACDEC_CONCEAL) == 0) as i32,
                        self_.flags[stream_index],
                        &self_.conceal_common_data,
                        spec,
                    )
                };
                if conceal_apply_return_code == -1 {
                    frame_ok_but_conceal = 1;
                }

                if flags & AACDEC_INTR != 0 {
                    /* Reset DRC control data for this channel */
                    aac_decoder_drc_init_channel_data(Some(&mut sci.drc_data));
                }
                if flags & AACDEC_CLRHIST != 0 && self_.flags[0] & AC_USAC == 0 {
                    /* Reset DRC control data for this channel */
                    aac_decoder_drc_init_channel_data(Some(&mut sci.drc_data));
                }

                /* The DRC module demands to be called with the gain field holding the gain
                 * scale. */
                self_.ext_gain[0] = AACDEC_DRC_GAIN_SCALING as FixpDbl;

                /* DRC processing */
                if let Some(drc) = self_.h_drc_info.as_deref_mut() {
                    let mut drc_data = sci.drc_data;
                    aac_decoder_drc_apply(
                        drc,
                        self_.h_sbr_decoder.as_deref_mut(),
                        ci,
                        &mut drc_data,
                        Some(&mut self_.ext_gain[0]),
                        c,
                        self_.stream_info.aac_samples_per_frame,
                        self_.sbr_enabled as i32,
                        spec,
                    );
                    sci.drc_data = drc_data;
                }

                if time_data_size < time_data_channel_offset.wrapping_mul(self_.aac_channels) {
                    error_status = AAC_DEC_OUTPUT_BUFFER_TOO_SMALL;
                    break;
                }
                let out = tail_mut(p_time_data, offset.max(0) as usize);
                if self_.flush_status != 0 && (self_.flush_cnt > 0) && (flags & AACDEC_CONCEAL) == 0
                {
                    let n =
                        (self_.stream_info.aac_samples_per_frame.max(0) as usize).min(out.len());
                    out[..n].fill(0);
                } else {
                    match ci.render_mode {
                        AACDEC_RENDER_IMDCT => {
                            let mut tmp_wb: Option<Box<CWorkBufferCore1>> = None;
                            let wb1: &mut CWorkBufferCore1 =
                                match self_.p_work_buffer_core1.as_deref_mut() {
                                    Some(w) => w,
                                    None => tmp_wb.insert(Box::<CWorkBufferCore1>::default()),
                                };
                            cblock_frequency_to_time(
                                sci,
                                ci,
                                out,
                                self_.stream_info.aac_samples_per_frame as i16,
                                (self_.frame_ok != 0
                                    && (flags & AACDEC_CONCEAL) == 0
                                    && frame_ok_but_conceal == 0)
                                    as i32,
                                &mut wb1.buf[..], // pWorkBuffer1: the whole union (see the callee)
                                self_.aac_out_data_headroom,
                                self_.el_flags[el],
                                el_ch,
                                spec,
                            );

                            self_.ext_gain_delay = self_.stream_info.aac_samples_per_frame as u32;
                        }
                        AACDEC_RENDER_ELDFB => {
                            cblock_frequency_to_time_low_delay(
                                sci,
                                ci,
                                out,
                                self_.stream_info.aac_samples_per_frame as i16,
                                spec,
                            );
                            self_.ext_gain_delay = ((self_.stream_info.aac_samples_per_frame * 2
                                - self_.stream_info.aac_samples_per_frame / 2
                                - 1)
                                / 2) as u32;
                        }
                        AACDEC_RENDER_LPD => {
                            let mut tmp_wb: Option<Box<CWorkBufferCore1>> = None;
                            let wb1: &mut CWorkBufferCore1 =
                                match self_.p_work_buffer_core1.as_deref_mut() {
                                    Some(w) => w,
                                    None => tmp_wb.insert(Box::<CWorkBufferCore1>::default()),
                                };
                            clpd_render_time_signal(
                                sci,
                                ci,
                                out,
                                self_.stream_info.aac_samples_per_frame,
                                &self_.sampling_rate_info[stream_index],
                                (self_.frame_ok != 0
                                    && (flags & AACDEC_CONCEAL) == 0
                                    && frame_ok_but_conceal == 0)
                                    as u32,
                                self_.aac_out_data_headroom,
                                flags,
                                self_.flags[stream_index],
                                spec,
                                wb1,
                            );

                            self_.ext_gain_delay = self_.stream_info.aac_samples_per_frame as u32;
                        }
                        _ => {
                            error_status = AAC_DEC_UNKNOWN;
                        }
                    }
                }
                /* TimeDomainFading */
                if cconceal_td_fading_applied[cu] == 0 {
                    if let Some(sci_c) = self_.p_aac_decoder_static_channel_info[cu].as_deref_mut()
                    {
                        cconceal_td_fading_applied[cu] = cconcealment_td_fading(
                            self_.stream_info.aac_samples_per_frame,
                            sci_c,
                            &self_.conceal_common_data,
                            self_.aac_out_data_headroom,
                            tail_mut(p_time_data, offset.max(0) as usize),
                        );
                    }
                    if c + 1 < 8 && c < aac_channels - 1 {
                        /* update next TDNoise Seed to avoid muting in case of Parametric Stereo */
                        let seed = self_.p_aac_decoder_static_channel_info[cu]
                            .as_deref()
                            .map(|s| s.concealment_info.td_noise_seed);
                        if let (Some(seed), Some(next)) =
                            (seed, self_.p_aac_decoder_static_channel_info[cu + 1].as_deref_mut())
                        {
                            next.concealment_info.td_noise_seed = seed;
                        }
                    }
                }

                c += 1;
                c_idx += 1;
                el_ch += 1;
            }
        }

        if self_.flags[stream_index] & AC_USAC != 0 {
            let mut bs_pseudo_lr = 0;
            mpeg_surround_decoder_is_pseudo_lr(
                self_.p_mpeg_surround_decoder.as_deref(),
                &mut bs_pseudo_lr,
            );
            /* ISO/IEC 23003-3, 7.11.2.6 Modification of core decoder output (pseudo LR) */
            if (aac_channels == 2) && bs_pseudo_lr != 0 {
                let inv_sqrt2: FixpSgl = fl2fxconst_sgl(0.707106781186547f32 as f64);
                let offset2 = time_data_channel_offset.max(0) as usize;

                for i in 0..self_.stream_info.aac_samples_per_frame.max(0) as usize {
                    let (Some(&l0), Some(&r0)) = (p_time_data.get(i), p_time_data.get(i + offset2))
                    else {
                        break;
                    };
                    let l = f_mult(l0, inv_sqrt2);
                    let r = f_mult(r0, inv_sqrt2);
                    p_time_data[i] = l.wrapping_add(r);
                    p_time_data[i + offset2] = l.wrapping_sub(r);
                }
            }
        }

        /* Extract DRC control data and map it to channels (with bitstream delay) */
        let mapped = aac_decoder_drc_epilog(
            self_.h_drc_info.as_deref_mut(),
            bs,
            &mut self_.p_aac_decoder_static_channel_info,
            self_.pce.element_instance_tag,
            &drc_ch_map,
            aac_channels,
        );
        if mapped > 0 {
            if self_.flags[stream_index] & (AC_USAC | AC_RSV603DA) == 0 {
                /* If at least one DRC thread has been mapped to a channel there was DRC data in
                 * the bitstream. */
                self_.flags[stream_index] |= AC_DRC_PRESENT;
            } else {
                error_status = AAC_DEC_UNSUPPORTED_FORMAT;
            }
        }
        if self_.flags[stream_index] & (AC_USAC | AC_RSV603DA) != 0 {
            if let Some(d) = self_.h_drc_info.as_deref_mut() {
                aac_decoder_drc_disable(d);
            }
        }
    }

    /* Add additional concealment delay */
    self_.stream_info.output_delay = self_.stream_info.output_delay.wrapping_add(
        cconcealment_get_delay(Some(&self_.conceal_common_data))
            .wrapping_mul(self_.stream_info.aac_samples_per_frame as u32),
    );

    /* Map DRC data to StreamInfo structure */
    aac_decoder_drc_get_info(
        self_.h_drc_info.as_deref(),
        Some(&mut self_.stream_info.drc_pres_mode),
        Some(&mut self_.stream_info.drc_prog_ref_lev),
    );

    /* Reorder channel type information tables.  */
    if self_.flags[0] & AC_RSV603DA == 0 {
        let types = self_.stream_info.p_channel_type;
        let idx = self_.stream_info.p_channel_indices;

        for c in 0..aac_channels.clamp(0, 8) as usize {
            let map_value = fdk_ch_map_descr_get_map_value(
                &self_.map_descr,
                c as u8,
                self_.ch_map_index as u32,
            ) as usize;
            // The map values of the channel maps are below 8 (validated descriptors).
            if map_value < 8 {
                self_.stream_info.p_channel_type[map_value] = types[c];
                self_.stream_info.p_channel_indices[map_value] = idx[c];
            }
        }
    }

    self_.block_number += 1;

    error_status
}

/// The `pComData` of the element in the channels `a..a + n`: that of its first channel, and that
/// of its second channel where it is a different allocation (`com_r` of
/// [`c_channel_element_read`]; see [`crate::channel`]), else `None`.
pub(crate) fn element_com_data<'a>(
    com_data: &'a mut [Box<CAacDecoderCommonData>],
    p_com_data: &[Option<usize>; NCH],
    a: usize,
    n: usize,
) -> (Option<&'a mut CAacDecoderCommonData>, Option<&'a mut CAacDecoderCommonData>) {
    let i0 = p_com_data.get(a).copied().flatten();
    let i1 = if n == 2 { p_com_data.get(a + 1).copied().flatten() } else { None };
    match (i0, i1) {
        (Some(i), Some(j)) if i != j => {
            let (lo, hi) = com_data.split_at_mut(i.max(j));
            let (x, y) = (&mut *lo[i.min(j)], &mut *hi[0]);
            if i < j {
                (Some(x), Some(y))
            } else {
                (Some(y), Some(x))
            }
        }
        // Without a pComData of the second channel the reference has no defined result; the
        // port passes None.
        (Some(i), _) => (Some(&mut *com_data[i]), None),
        (None, _) => (None, None),
    }
}

/// `CChannelElement_Read()` of the channel element of the channels `a..a + el_channels`
/// (the call of `CAacDecoder_DecodeFrame()`).
pub(crate) fn read_channel_element(
    self_: &mut AacDecoderInstance,
    bs: &mut FdkBitStream,
    a: usize,
    el_channels: i32,
    element_count: usize,
) -> AacDecoderError {
    let n = el_channels.max(0) as usize;
    let mut p = channels_mut(&mut self_.p_aac_decoder_channel_info, a, n);
    let mut ps = channels_mut(&mut self_.p_aac_decoder_static_channel_info, a, n);
    if p.len() < n || ps.len() < n {
        // The channel infos below ascChannels exist (C dereferences them).
        return AAC_DEC_UNKNOWN;
    }
    let cpe_idx = ps[0].p_cpe_static_data;
    let cpe = cpe_idx.and_then(|i| self_.cpe_static_data.get_mut(i)).and_then(|c| c.as_deref_mut());

    let mut fb_com: Option<Box<CAacDecoderCommonData>> = None;
    let (com, com_r) = element_com_data(&mut self_.com_data, &self_.p_com_data, a, n);
    let com: &mut CAacDecoderCommonData = match com {
        Some(c) => c,
        None => fb_com.insert(Box::<CAacDecoderCommonData>::default()),
    };
    let mut fb_com_static: Option<Box<CAacDecoderCommonStaticData>> = None;
    let com_static: &mut CAacDecoderCommonStaticData =
        match self_.p_com_static_data.get(a).copied().flatten() {
            Some(i) => &mut self_.com_static_data[i],
            None => fb_com_static.insert(Box::<CAacDecoderCommonStaticData>::default()),
        };
    let mut empty: [FixpDbl; 0] = [];
    let spec: &mut [FixpDbl] = match self_.work_buffer_core2.as_deref_mut() {
        Some(wb2) => tail_mut(wb2, a * 1024),
        None => &mut empty,
    };

    c_channel_element_read(
        bs,
        &mut p,
        Some(&mut ps),
        self_.stream_info.aot,
        &self_.sampling_rate_info[0],
        self_.flags[0],
        self_.el_flags[element_count],
        self_.stream_info.aac_samples_per_frame as u32,
        el_channels as u8,
        self_.stream_info.ep_config,
        self_.h_input.as_deref_mut(),
        spec,
        com,
        com_r,
        com_static,
        cpe,
    )
}

/// The coupling channel element (CCE) case of `CAacDecoder_DecodeFrame()`: the element is
/// parsed into a dummy channel and dropped (decoding of CCE is not supported).
///
/// C builds the dummy channel on the stack (`tmpAacDecoderChannelInfo`, `commonData`,
/// `commonStaticData`, `workBufferCore1`) and sets `pDynData`, `pComData`, `pComStaticData`,
/// `pSpectralCoefficient`, `granuleLength` and the PNS data (`CPns_ResetData()`). The port
/// starts from zeroed structures.
pub(crate) fn read_coupling_channel_element(
    self_: &mut AacDecoderInstance,
    bs: &mut FdkBitStream,
    previous_element_index: &mut u8,
) -> AacDecoderError {
    let mut common_data = Box::<CAacDecoderCommonData>::default();
    let mut common_static_data = CAacDecoderCommonStaticData::default();
    /* memory for spectral lines temporal on scratch */
    let mut mdct_spec = vec![0 as FixpDbl; 1024];

    /* create dummy channel for CCE parsing on stack */
    let mut tmp_aac_decoder_channel_info = Box::<CAacDecoderChannelInfo>::default();

    /* Assume AAC-LC */
    tmp_aac_decoder_channel_info.granule_length = self_.stream_info.aac_samples_per_frame / 8;
    /* Reset PNS data. */
    cpns_reset_data(
        &mut tmp_aac_decoder_channel_info.data.aac.pns_data,
        &mut common_data.pns_inter_channel_data,
    );
    /* do CCE parsing */
    let error_status = {
        let mut p = [&mut *tmp_aac_decoder_channel_info];
        c_channel_element_read(
            bs,
            &mut p,
            None,
            self_.stream_info.aot,
            &self_.sampling_rate_info[0],
            self_.flags[0],
            AC_EL_GA_CCE,
            self_.stream_info.aac_samples_per_frame as u32,
            1,
            self_.stream_info.ep_config,
            self_.h_input.as_deref_mut(),
            &mut mdct_spec,
            &mut common_data,
            None,
            &mut common_static_data,
            None,
        )
    };

    if error_status != 0 {
        self_.frame_ok = 0;
    }

    if self_.frame_ok != 0 {
        /* Lookup the element and decode it only if it belongs to the current program */
        if cprogram_config_lookup_element(
            &mut self_.pce,
            self_.stream_info.channel_config as u32,
            tmp_aac_decoder_channel_info.element_instance_tag as u32,
            0,
            &mut self_.ch_mapping,
            &mut self_.stream_info.p_channel_type,
            &mut self_.stream_info.p_channel_indices,
            8,
            previous_element_index,
            &mut self_.elements,
            ID_CCE,
        ) != 0
        {
            /* decoding of CCE not supported */
        } else {
            self_.frame_ok = 0;
        }
    }

    error_status
}

/// `CChannelElement_Decode()` of the channel element of the channels `a..a + el_channels`
/// (`scratch` is the joint stereo scratch buffer, `pTimeData`).
pub(crate) fn decode_channel_element(
    self_: &mut AacDecoderInstance,
    a: usize,
    el_channels: i32,
    element_count: usize,
    scratch: &mut [FixpDbl],
) {
    let n = el_channels.max(0) as usize;
    // *pAacDecoderChannelInfo[a]->data.aac.PnsData.currentSeed: the pnsCurrentSeed of the static
    // channel info it points to, which need not be channel a's (see
    // crate::channelinfo::aacdec_pns_types). Copied in and out: nothing else in
    // CChannelElement_Decode() reaches it. NULL (USAC, which applies no PNS): a dummy.
    let seed_ch = self_
        .p_aac_decoder_channel_info
        .get(a)
        .and_then(|c| c.as_deref())
        .and_then(|c| c.data.aac.pns_data.current_seed);
    let mut current_seed = seed_ch
        .and_then(|i| self_.p_aac_decoder_static_channel_info.get(i)?.as_deref())
        .map_or(0, |s| s.pns_current_seed);
    let mut p = channels_mut(&mut self_.p_aac_decoder_channel_info, a, n);
    let mut ps = channels_mut(&mut self_.p_aac_decoder_static_channel_info, a, n);
    if p.len() < n || ps.len() < n || n == 0 {
        // The channel infos below aacChannels exist (C dereferences them).
        return;
    }
    let cpe_idx = ps[0].p_cpe_static_data;
    let cpe = cpe_idx.and_then(|i| self_.cpe_static_data.get_mut(i)).and_then(|c| c.as_deref_mut());

    let mut fb_com: Option<Box<CAacDecoderCommonData>> = None;
    let (com, com_r) = element_com_data(&mut self_.com_data, &self_.p_com_data, a, n);
    let com: &mut CAacDecoderCommonData = match com {
        Some(c) => c,
        None => fb_com.insert(Box::<CAacDecoderCommonData>::default()),
    };
    let mut fb_com_static: Option<Box<CAacDecoderCommonStaticData>> = None;
    let com_static: &mut CAacDecoderCommonStaticData =
        match self_.p_com_static_data.get(a).copied().flatten() {
            Some(i) => &mut self_.com_static_data[i],
            None => fb_com_static.insert(Box::<CAacDecoderCommonStaticData>::default()),
        };
    let mut empty: [FixpDbl; 0] = [];
    let spec: &mut [FixpDbl] = match self_.work_buffer_core2.as_deref_mut() {
        Some(wb2) => tail_mut(wb2, a * 1024),
        None => &mut empty,
    };

    c_channel_element_decode(
        &mut p,
        &mut ps,
        &self_.sampling_rate_info[0],
        self_.flags[0],
        self_.el_flags[element_count],
        el_channels,
        spec,
        com,
        com_r,
        com_static,
        cpe,
        scratch,
        &mut current_seed,
    );
    if let Some(s) =
        seed_ch.and_then(|i| self_.p_aac_decoder_static_channel_info.get_mut(i)?.as_deref_mut())
    {
        s.pns_current_seed = current_seed;
    }
}

/// `CAacDecoder_GetStreamInfo(self)`: the stream information.
pub fn caac_decoder_get_stream_info(self_: Option<&AacDecoderInstance>) -> Option<&CStreamInfo> {
    self_.map(|s| &s.stream_info)
}
