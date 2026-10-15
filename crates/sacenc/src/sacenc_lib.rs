// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libSACenc/include/sacenc_lib.h, libSACenc/src/sacenc_lib.cpp.

//! Port of `sacenc_lib.h` and `sacenc_lib.cpp`: the API of the MPEG Surround encoder (the
//! low delay 212 encoder of ELDv2: stereo in, mono downmix and spatial parameters out).
//!
//! # API
//!
//! The functions keep the C parameters: [`fdk_sacenc_open`], [`fdk_sacenc_set_param`],
//! [`fdk_sacenc_init`], [`fdk_sacenc_encode`], [`fdk_sacenc_get_info`],
//! [`fdk_sacenc_get_lib_info`] and [`fdk_sacenc_close`]. A `NULL` handle is `None`; the
//! handle itself is `Option<Box<Mp4SpaceEncoder>>` (`HANDLE_MP4SPACE_ENCODER`).
//!
//! # Notes for the AAC encoder glue (`mps_main.cpp`)
//!
//! - `FDK_MpegsEnc_Init` sets `SACENC_LOWDELAY` 2, `SACENC_ENC_MODE` `SACENC_212`, the
//!   sampling rate, the time slots (`framelength >> 5` or `>> 6`), `SACENC_PARAM_BANDS` 15,
//!   `SACENC_TIME_DOM_DMX` 2, `SACENC_COARSE_QUANT` 0, `SACENC_QUANT_MODE` fine,
//!   `SACENC_TIME_ALIGNMENT` 0 and `SACENC_INDEPENDENCY_FACTOR` 20 with
//!   [`fdk_sacenc_set_param`], then calls [`fdk_sacenc_init`] with the core coder delay.
//!   The encoder only initializes successfully when the core coder delay lies in
//!   `frameLength / 2 ..= frameLength / 2 + 5` (the surround analysis buffer holds 5 samples).
//! - [`fdk_sacenc_encode`] is `unsafe`: it takes the buffer descriptors of the C API
//!   ([`FdkBufDescr`], raw pointers). The glue passes the same PCM buffer as input (two
//!   channels, not interleaved, `inputBufferSizePerChannel` apart) and as output (the mono
//!   downmix of `nFrameLength` samples is written over the start of the first channel after all
//!   input has been read), and its `sacOutBuffer` (after the first byte) as the bitstream
//!   output, which is always `outBufDesc->ppBase[1]` (see the safety section of
//!   [`fdk_sacenc_encode`]).
//! - `FDK_MpegsEnc_WriteSpatialSpecificConfig` and `FDK_MpegsEnc_GetDelay`/`GetDecDelay` use
//!   [`fdk_sacenc_get_info`]; [`Mp4SpaceEncInfo::p_ssc_buf`] borrows the SpatialSpecificConfig
//!   buffer of the encoder (`nSscSizeBits` and the bytes `pSsc`).
//! - `FDK_MpegsEnc_GetLibInfo` uses [`fdk_sacenc_get_lib_info`].
//!
//! # Memory
//!
//! The buffers are allocated with the maximum sizes of the C code when the encoder is opened
//! (48 kHz, 16 time slots: 64 bands, 1024 samples) and kept across initializations, so their
//! contents from earlier initializations are reproduced. The buffers
//! that the C code does not allocate in 212 mode (`ppTimeSigOut__FDK`,
//! `pOutputDelayBuffer__FDK`, `pppProcDataIn__FDK`, `ppBitstreamDelayBuffer`) are not
//! allocated either; the code paths that use them are not reachable (the mode can only be set
//! to 212).
//!
//! # Deviations
//!
//! Where the reference has no defined result (PORTING.md, "Deviations from the reference"):
//!
//! - [`fdk_sacenc_encode`]: the port copies what fits into the buffers and reads 0 past them.
//! - Divisions by zero (`% nChInWithDmx`, `% nBitstreamDelayBuffer`, `/ pEleSize`) give the
//!   arm64 result (quotient 0, remainder = dividend) on both variants.
//! - Buffer descriptor lookups: the port uses `NULL` (and a size of 0).
//! - The non-212 code paths return `SACENC_INVALID_HANDLE`.

use core::ffi::c_void;

use fdk_aac_rs_fdk::audio::*;
use fdk_aac_rs_fdk::common_fix::*;
use fdk_aac_rs_fdk::core::fdk_tools_get_lib_info;
use fdk_aac_rs_fdk::machine_type::IntPcm;
use fdk_aac_rs_fdk::qmf::*;

use crate::sacenc_bitstream::*;
use crate::sacenc_const::*;
use crate::sacenc_delay::*;
use crate::sacenc_dmx_tdom_enh::*;
use crate::sacenc_filter::*;
use crate::sacenc_framewindowing::*;
use crate::sacenc_onsetdetect::*;
use crate::sacenc_paramextract::*;
use crate::sacenc_staticgain::*;
use crate::sacenc_tree::*;
use crate::sacenc_vectorfunctions::*;

mod tables;
pub use tables::*;

/// `FDK_SACENC_ERROR`
pub type FdkSacencError = i32;
/// No error happened. All fine.
pub const SACENC_OK: FdkSacencError = 0x0000_0000;
/// Handle passed to function call was invalid.
pub const SACENC_INVALID_HANDLE: FdkSacencError = 0x0000_0080;
/// Memory allocation failed.
pub const SACENC_MEMORY_ERROR: FdkSacencError = 0x0000_0800;
/// General initialization error.
pub const SACENC_INIT_ERROR: FdkSacencError = 0x0000_8000;
/// The encoding process was interrupted by an unexpected error.
pub const SACENC_ENCODE_ERROR: FdkSacencError = 0x0008_0000;
/// Invalid runtime parameter.
pub const SACENC_PARAM_ERROR: FdkSacencError = 0x0080_0000;
/// Parameter not available.
pub const SACENC_UNSUPPORTED_PARAMETER: FdkSacencError = 0x0080_0001;
/// Configuration not provided.
pub const SACENC_INVALID_CONFIG: FdkSacencError = 0x0080_0002;
/// Unknown error.
pub const SACENC_UNKNOWN_ERROR: FdkSacencError = 0x0800_0000;

/// `MP4SPACEENC_MODE`
pub type Mp4SpaceEncMode = i32;
pub const SACENC_INVALID_MODE: Mp4SpaceEncMode = 0;
pub const SACENC_212: Mp4SpaceEncMode = 8;
pub const SACENC_ESCAPE: Mp4SpaceEncMode = 15;

/// `MP4SPACEENC_BANDS_CONFIG`
pub type Mp4SpaceEncBandsConfig = i32;
pub const SACENC_BANDS_INVALID: Mp4SpaceEncBandsConfig = 0;
pub const SACENC_BANDS_4: Mp4SpaceEncBandsConfig = 4;
pub const SACENC_BANDS_5: Mp4SpaceEncBandsConfig = 5;
pub const SACENC_BANDS_7: Mp4SpaceEncBandsConfig = 7;
pub const SACENC_BANDS_9: Mp4SpaceEncBandsConfig = 9;
pub const SACENC_BANDS_12: Mp4SpaceEncBandsConfig = 12;
pub const SACENC_BANDS_15: Mp4SpaceEncBandsConfig = 15;
pub const SACENC_BANDS_23: Mp4SpaceEncBandsConfig = 23;

/// `MP4SPACEENC_QUANTMODE`
pub type Mp4SpaceEncQuantMode = i32;
pub const SACENC_QUANTMODE_INVALID: Mp4SpaceEncQuantMode = -1;
pub const SACENC_QUANTMODE_FINE: Mp4SpaceEncQuantMode = 0;
pub const SACENC_QUANTMODE_EBQ1: Mp4SpaceEncQuantMode = 1;
pub const SACENC_QUANTMODE_EBQ2: Mp4SpaceEncQuantMode = 2;
pub const SACENC_QUANTMODE_RSVD3: Mp4SpaceEncQuantMode = 3;

/// `MP4SPACEENC_DMX_GAIN`
pub type Mp4SpaceEncDmxGain = i32;
pub const SACENC_DMXGAIN_INVALID: Mp4SpaceEncDmxGain = -1;
pub const SACENC_DMXGAIN_0_dB: Mp4SpaceEncDmxGain = 0;
pub const SACENC_DMXGAIN_1_5_dB: Mp4SpaceEncDmxGain = 1;
pub const SACENC_DMXGAIN_3_dB: Mp4SpaceEncDmxGain = 2;
pub const SACENC_DMXGAIN_4_5_dB: Mp4SpaceEncDmxGain = 3;
pub const SACENC_DMXGAIN_6_dB: Mp4SpaceEncDmxGain = 4;
pub const SACENC_DMXGAIN_7_5_dB: Mp4SpaceEncDmxGain = 5;
pub const SACENC_DMXGAIN_9_dB: Mp4SpaceEncDmxGain = 6;
pub const SACENC_DMXGAIN_12_dB: Mp4SpaceEncDmxGain = 7;

/// `SPACEENC_PARAM`
pub type SpaceEncParam = i32;
/// Configure lowdelay MPEG Surround (0: off, 1: on, 2: on including keep frame).
pub const SACENC_LOWDELAY: SpaceEncParam = 0;
/// Configure encoder tree mode (`MP4SPACEENC_MODE`).
pub const SACENC_ENC_MODE: SpaceEncParam = 1;
/// Configure encoder sampling rate.
pub const SACENC_SAMPLERATE: SpaceEncParam = 2;
/// Configure number of slots per spatial frame.
pub const SACENC_FRAME_TIME_SLOTS: SpaceEncParam = 3;
/// Configure number of parameter bands (`MP4SPACEENC_BANDS_CONFIG`).
pub const SACENC_PARAM_BANDS: SpaceEncParam = 4;
/// Configure time domain downmix (0: none, 1: static, 2: enhanced).
pub const SACENC_TIME_DOM_DMX: SpaceEncParam = 5;
/// Configure downmix gain (`MP4SPACEENC_DMX_GAIN`).
pub const SACENC_DMX_GAIN: SpaceEncParam = 6;
/// Use coarse parameter quantization.
pub const SACENC_COARSE_QUANT: SpaceEncParam = 7;
/// Configure quantization mode (`MP4SPACEENC_QUANTMODE`).
pub const SACENC_QUANT_MODE: SpaceEncParam = 8;
/// Configure time alignment in samples.
pub const SACENC_TIME_ALIGNMENT: SpaceEncParam = 9;
/// Configure the independency count.
pub const SACENC_INDEPENDENCY_COUNT: SpaceEncParam = 10;
/// How often the independency flag is set.
pub const SACENC_INDEPENDENCY_FACTOR: SpaceEncParam = 11;
pub const SACENC_NONE: SpaceEncParam = 12;

/// `MPEG4SPACEENC_SSCBUF`: the SpatialSpecificConfig.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Mpeg4SpaceEncSscBuf {
    /// `nSscSizeBits`: number of valid bits in `p_ssc`
    pub n_ssc_size_bits: i32,
    /// `pSsc`: the SpatialSpecificConfig in binary format (`MAX_SSC_BYTES` bytes)
    pub p_ssc: Vec<u8>,
}

/// `MP4SPACEENC_INFO`
#[derive(Clone, Copy, Debug, Default)]
pub struct Mp4SpaceEncInfo<'a> {
    /// Configured sampling rate.
    pub n_sample_rate: i32,
    /// Frame length in samples.
    pub n_samples_frame: i32,
    /// Number of expected audio input channels.
    pub n_total_input_channels: i32,
    /// Delay of the downmixed signal.
    pub n_dmx_delay: i32,
    /// Delay of the whole en-/decoded signal, including core-coder delay.
    pub n_codec_delay: i32,
    /// Delay added by the MP4SPACE decoder.
    pub n_decoder_delay: i32,
    /// Delay of the payload.
    pub n_payload_delay: i32,
    /// Number of dmx frames to discard for alignment with bitstream.
    pub n_discard_out_frames: i32,
    /// `pSscBuf`: the SpatialSpecificConfig of the encoder.
    pub p_ssc_buf: Option<&'a Mpeg4SpaceEncSscBuf>,
}

/// `SACENC_InArgs`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SacencInArgs {
    /// Number of valid input audio samples (multiple of input channels).
    pub n_input_samples: i32,
    /// Size of input buffer (input audio samples) per channel.
    pub input_buffer_size_per_channel: u32,
    /// 0: in blocks, 1: interleaved.
    pub is_input_interleaved: u32,
}

/// `SACENC_OutArgs`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SacencOutArgs {
    /// Number of valid payload bits generated during the call.
    pub n_output_bits: i32,
    /// Number of valid output audio samples generated during the call.
    pub n_output_samples: i32,
    /// Number of input audio samples consumed in the call.
    pub n_samples_consumed: u32,
}

/// `SACENC_LIB_VL0`
const SACENC_LIB_VL0: i32 = 2;
/// `SACENC_LIB_VL1`
const SACENC_LIB_VL1: i32 = 0;
/// `SACENC_LIB_VL2`
const SACENC_LIB_VL2: i32 = 0;
/// `SACENC_LIB_TITLE`
const SACENC_LIB_TITLE: &str = "MPEG Surround Encoder";
/// `SACENC_LIB_BUILD_DATE`: `__DATE__` in C (empty in the port).
const SACENC_LIB_BUILD_DATE: &str = "";
/// `SACENC_LIB_BUILD_TIME`: `__TIME__` in C (empty in the port).
const SACENC_LIB_BUILD_TIME: &str = "";

/// `MAX_MPEGS_BYTES`
pub const MAX_MPEGS_BYTES: i32 = 1 << 14;
/// `MAX_SSC_BYTES`
pub const MAX_SSC_BYTES: i32 = 1 << 6;

/// `MAX_SPACE_TREE_CHANNELS`
const MAX_SPACE_TREE_CHANNELS: usize = 2;
/// `NUM_KEEP_WINDOWS`
const NUM_KEEP_WINDOWS: i32 = 3;

/// `MP4SPACEENC_SETUP`: the user configuration.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mp4SpaceEncSetup {
    pub enc_mode: Mp4SpaceEncMode,
    pub n_param_bands: Mp4SpaceEncBandsConfig,
    pub quant_mode: Mp4SpaceEncQuantMode,
    pub b_use_coarse_quant: u8,
    pub b_ld_mode: u8,
    pub b_time_domain_dmx: u8,
    pub sample_rate: u32,
    /// e.g. 32 when used with HE-AAC
    pub frame_time_slots: u32,
    /// how often should we set the independency flag
    pub independency_factor: u32,
    /// additional delay for downmix
    pub time_alignment: i32,
}

/// `struct ENC_CONFIG_SETUP`: the configuration the instance is allocated for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EncConfigSetup {
    pub b_enc_mode_212: u8,
    pub max_hybrid_in_static_slots: u8,
    pub max_samplingrate: i32,
    pub max_analysis_length_time_slots: i32,
    pub max_hybrid_bands: i32,
    pub max_qmf_bands: i32,
    pub max_ch_in: i32,
    pub max_frame_time_slots: i32,
    pub max_frame_length: i32,
    pub max_ch_out: i32,
    pub max_ch_tot_out: i32,
}

/// `struct MP4SPACE_ENCODER`. The sub-modules are owned (`HANDLE_*` are never `NULL` after a
/// successful open). The buffers of the C code that are not allocated in 212 mode are left
/// out (see the [module documentation](self)).
#[derive(Clone, Debug)]
pub struct Mp4SpaceEncoder {
    pub user: Mp4SpaceEncSetup,

    /// describe allocated instance
    pub setup: EncConfigSetup,

    /// Windowing, only created+updated, but not used
    pub h_frame_window: Box<FrameWindow>,
    /// Input Buffer Handling
    pub n_samples_valid: i32,

    // Routing Sensible Switches/Variables
    pub n_param_bands: Mp4SpaceEncBandsConfig,
    pub use_time_dom_downmix: u8,

    // not Routing Sensible Switches/Varibles - must be contained in Check
    pub enc_mode: Mp4SpaceEncMode,
    pub b_enc_mode_212_only: u8,

    // not Routing Sensible Switches/Varibles + lower Classes
    pub use_frame_keep: u8,
    pub independency_factor: u32,
    pub n_sample_rate: u32,
    pub n_input_channels: u8,
    pub n_output_channels: u8,
    /// e.g. 32 when used with HE-AAC
    pub n_frame_time_slots: u8,
    pub n_qmf_bands: u8,
    pub n_hybrid_bands: u8,
    /// number of output waveform samples/channel/frame
    pub n_frame_length: u32,

    // not Routing Sensible Switches/Varibles + lower Classes, secondary computed
    pub n_samples_next: i32,
    pub n_analysis_length_time_slots: i32,
    pub n_analysis_lookahead_time_slots: i32,
    pub n_update_hybrid_position_time_slots: i32,
    /// `pnOutputBits`: `MAX_BITSTREAM_DELAY` values
    pub pn_output_bits: Vec<i32>,
    pub n_input_delay: i32,
    pub n_output_buffer_delay: i32,
    pub n_surround_analysis_buffer_delay: i32,
    pub n_bitstream_delay_buffer: i32,
    pub n_bitstream_buffer_read: i32,
    pub n_bitstream_buffer_write: i32,
    pub n_discard_out_frames: i32,
    pub avoid_keep: i32,

    // not Routing Sensible Switches/Varibles -> moved to lower Classes
    pub use_coarse_quant_cld: u8,
    pub use_coarse_quant_icc: u8,
    pub use_coarse_quant_cpc: u8,
    pub use_coarse_quant_arb_dmx: u8,
    pub quant_mode: Mp4SpaceEncQuantMode,
    pub core_coder_delay: i32,
    pub time_alignment: i32,

    // Local Processing Variables
    pub independency_count: i32,
    pub independency_flag: i32,
    /// `ppTrCurrPos`: `maxChIn` x `MAX_NUM_TRANS`
    pub pp_tr_curr_pos: Vec<[i32; MAX_NUM_TRANS]>,
    pub tr_prev_pos: [i32; 2 * MAX_NUM_TRANS],

    pub frame_win_list: FrameWinList,
    pub save_frame: SpatialFrame,

    // Module-Handles
    pub space_tree_setup: SpaceTreeSetup,
    pub ssc_buf: Mpeg4SpaceEncSscBuf,
    /// `pFrameWindowAna__FDK`: `maxAnalysisLengthTimeSlots` values each
    pub p_frame_window_ana_fdk: [Vec<FixpWin>; MAX_NUM_PARAMS],
    /// `phQmfFiltIn__FDK`
    pub ph_qmf_filt_in_fdk: Vec<QmfFilterBank>,
    /// `phQmfFiltIn__FDK[ch]->FilterStates`: `2 * 5 * maxQmfBands` `FIXP_QAS` (`FIXP_PCM`)
    pub qmf_filter_states: Vec<Vec<FixpSgl>>,
    pub ph_dc_filter_sig_in: Vec<Box<DcFilter>>,
    pub ph_onset: Vec<Box<OnsetDetect>>,
    pub h_space_tree: Box<SpaceTree>,
    pub h_bitstream_formatter: Box<BsfInstance>,
    pub h_static_gain_config: Box<StaticGainConfig>,
    pub h_static_gain: Box<StaticGain>,
    pub h_delay: Box<Delay>,

    /// enhanced time domain downmix (for stereo input)
    pub h_enhanced_time_dmx: Box<EnhancedTimeDomainDmx>,

    // Data Buffers
    /// `ppTimeSigIn__FDK`: `maxChIn` x (`maxFrameLength + MAX_DELAY_SURROUND_ANALYSIS`)
    pub pp_time_sig_in_fdk: Vec<Vec<IntPcm>>,
    /// `ppTimeSigDelayIn__FDK`: `maxChIn` x `MAX_DELAY_SURROUND_ANALYSIS`
    pub pp_time_sig_delay_in_fdk: Vec<Vec<IntPcm>>,
    /// `pppHybridIn__FDK`: `maxChIn` x (`maxAnalysisLengthTimeSlots - maxFrameTimeSlots / 2`)
    /// x `maxHybridBands`
    pub ppp_hybrid_in_fdk: Vec<Vec<Vec<FixpDpk>>>,
    /// `pppHybridInStatic__FDK`: `maxChIn` x `maxHybridInStaticSlots` x `maxHybridBands`
    pub ppp_hybrid_in_static_fdk: Vec<Vec<Vec<FixpDpk>>>,

    /// `pParameterBand2HybridBandOffset`: `MAX_NUM_PARAM_BANDS` values
    pub p_parameter_band2_hybrid_band_offset: Vec<u8>,
    pub static_gain_scale: i32,

    /// `pEncoderInputChScale`: `maxChIn` values
    pub p_encoder_input_ch_scale: Vec<i32>,
    /// `staticTimeDomainDmxInScale`: `maxChIn` values
    pub static_time_domain_dmx_in_scale: Vec<i32>,
}

/// `HANDLE_MP4SPACE_ENCODER`
pub type HandleMp4SpaceEncoder = Option<Box<Mp4SpaceEncoder>>;

/// `INT / INT` as arm64 computes it (`sdiv`: 0 for a divisor of 0).
#[inline]
fn div_i(a: i32, b: i32) -> i32 {
    if b == 0 {
        0
    } else {
        a.wrapping_div(b)
    }
}

/// `INT % INT` as arm64 computes it (`sdiv`, `msub`: the dividend for a divisor of 0).
#[inline]
fn rem_i(a: i32, b: i32) -> i32 {
    if b == 0 {
        a
    } else {
        a.wrapping_rem(b)
    }
}

/// `UINT / UINT` as arm64 computes it (`udiv`: 0 for a divisor of 0).
#[inline]
fn div_u(a: u32, b: u32) -> u32 {
    a.checked_div(b).unwrap_or(0)
}

/// `mp4SpaceEnc_GetNumQmfBands(nSampleRate)` (static in C)
pub fn mp4_space_enc_get_num_qmf_bands(n_sample_rate: u32) -> u32 {
    if n_sample_rate < 27713 {
        32
    } else if n_sample_rate < 55426 {
        64
    } else {
        0
    }
}

/// `updateQmfFlags(flags, keepStates)` (static in C)
pub fn update_qmf_flags(flags: u32, keep_states: i32) -> u32 {
    let mut qmf_flags = flags;

    qmf_flags &= !QMF_FLAG_LP;
    qmf_flags |= QMF_FLAG_MPSLDFB;
    qmf_flags = if keep_states != 0 {
        qmf_flags | QMF_FLAG_KEEP_STATES
    } else {
        qmf_flags & !QMF_FLAG_KEEP_STATES
    };

    qmf_flags
}

/// `freq2HybridBand(nFrequency, nSampleRate, nQmfBands)` (static in C): the hybrid band of a
/// frequency, -1 outside of the bands.
pub fn freq2_hybrid_band(n_frequency: u32, n_sample_rate: u32, n_qmf_bands: u32) -> i32 {
    // nQmfSlotWidth = (nSampleRate/2) / nQmfBands;
    // nQmfBand      = nFrequency / nQmfSlotWidth;
    let mut n_hybrid_band = -1;
    let mut scale = 0;
    let temp = f_div_norm(
        2u32.wrapping_mul(n_frequency).wrapping_mul(n_qmf_bands) as FixpDbl,
        n_sample_rate as FixpDbl,
        &mut scale,
    );
    let n_qmf_band = scale_value(temp, scale - (DFRACT_BITS - 1));

    if (n_qmf_band > -1) && (n_qmf_band < n_qmf_bands as i32) {
        // `qmf2qmf[nQmfBand]`: the bands are at most 64, inside of the 128 entries.
        n_hybrid_band = QMF2QMF.get(n_qmf_band as usize).map_or(0, |&b| b as i32);
    }

    n_hybrid_band
}

/// `getBufDescIdx(pBufDesc, type)` (static in C): the index of the buffer of the type, -1 if
/// there is none.
///
/// # Safety
/// `p_buf_desc.p_buf_type` must be valid for `num_bufs` reads.
pub unsafe fn get_buf_desc_idx(p_buf_desc: &FdkBufDescr, type_: u32) -> i32 {
    for i in 0..p_buf_desc.num_bufs as i32 {
        if *p_buf_desc.p_buf_type.add(i as usize) == type_ {
            return i;
        }
    }
    -1
}

/// `FDK_sacenc_open(phMp4SpaceEnc)`: opens a new instance of the MPEG Surround encoder.
pub fn fdk_sacenc_open(ph_mp4_space_enc: Option<&mut HandleMp4SpaceEncoder>) -> FdkSacencError {
    mp4_space_enc_create(ph_mp4_space_enc)
}

/// `mp4SpaceEnc_create(phMp4SpaceEnc)` (static in C)
fn mp4_space_enc_create(ph_mp4_space_enc: Option<&mut HandleMp4SpaceEncoder>) -> FdkSacencError {
    let Some(ph) = ph_mp4_space_enc else {
        return SACENC_INVALID_HANDLE;
    };

    let mut setup =
        EncConfigSetup { max_samplingrate: 48000, max_frame_time_slots: 16, ..Default::default() };
    setup.max_analysis_length_time_slots = 3 * setup.max_frame_time_slots;
    setup.max_qmf_bands = mp4_space_enc_get_num_qmf_bands(setup.max_samplingrate as u32) as i32;
    setup.max_hybrid_bands = setup.max_qmf_bands;
    setup.max_frame_length = setup.max_qmf_bands * setup.max_frame_time_slots;

    setup.max_ch_in = 2;
    setup.max_ch_out = 1;
    setup.max_ch_tot_out = setup.max_ch_out;
    setup.b_enc_mode_212 = 1;
    setup.max_hybrid_in_static_slots = 24;

    let max_ch_in = setup.max_ch_in as usize;

    // The sub-modules; their open functions only fail for a NULL handle.
    let mut h_static_gain_config = None;
    let mut h_enhanced_time_dmx = None;
    let mut h_space_tree = None;
    let mut h_frame_window = None;
    let mut h_static_gain = None;
    let mut h_bitstream_formatter = None;
    let mut h_delay = None;
    let mut dc = Vec::new();
    let mut onset = Vec::new();

    // Open Static Gain
    let mut error = fdk_sacenc_static_gain_open_config(Some(&mut h_static_gain_config));
    // enhanced time domain downmix (for stereo input)
    if error == SACENC_OK {
        error = fdk_sacenc_open_enhanced_time_domain_dmx(
            Some(&mut h_enhanced_time_dmx),
            setup.max_frame_length,
        );
    }
    // Create Space Tree first, to get number of in-/output channels
    if error == SACENC_OK {
        error = fdk_sacenc_space_tree_open(Some(&mut h_space_tree));
    }
    // Allocate DC Filter Struct for normal signal input
    for _ in 0..max_ch_in {
        let mut h = None;
        if error == SACENC_OK {
            error = fdk_sacenc_create_dc_filter(Some(&mut h));
        }
        dc.extend(h);
    }
    // Open Onset Detection
    for _ in 0..max_ch_in {
        let mut h = None;
        if error == SACENC_OK {
            error = fdk_sacenc_onset_detect_open(Some(&mut h), setup.max_frame_time_slots as u32);
        }
        onset.extend(h);
    }
    // Create Windowing
    if error == SACENC_OK {
        error = fdk_sacenc_frame_window_create(Some(&mut h_frame_window));
    }
    // Open static gain
    if error == SACENC_OK {
        error = fdk_sacenc_static_gain_open(Some(&mut h_static_gain));
    }
    // create bitstream encoder
    if error == SACENC_OK {
        error = fdk_sacenc_create_spatial_bitstream_encoder(Some(&mut h_bitstream_formatter));
    }
    if error == SACENC_OK {
        error = fdk_sacenc_delay_open(Some(&mut h_delay));
    }

    let (
        Some(h_static_gain_config),
        Some(h_enhanced_time_dmx),
        Some(h_space_tree),
        Some(h_frame_window),
        Some(h_static_gain),
        Some(h_bitstream_formatter),
        Some(h_delay),
        true,
    ) = (
        h_static_gain_config,
        h_enhanced_time_dmx,
        h_space_tree,
        h_frame_window,
        h_static_gain,
        h_bitstream_formatter,
        h_delay,
        error == SACENC_OK && dc.len() == max_ch_in && onset.len() == max_ch_in,
    )
    else {
        return if error == SACENC_OK { SACENC_MEMORY_ERROR } else { error };
    };

    let max_qmf = setup.max_qmf_bands as usize;
    let max_hyb = setup.max_hybrid_bands as usize;
    let hybrid = |slots: usize| vec![vec![vec![FixpDpk::default(); max_hyb]; slots]; max_ch_in];

    let mut h_enc = Box::new(Mp4SpaceEncoder {
        user: Mp4SpaceEncSetup::default(),
        setup,
        h_frame_window,
        n_samples_valid: 0,
        n_param_bands: 0,
        use_time_dom_downmix: 0,
        enc_mode: 0,
        b_enc_mode_212_only: 0,
        use_frame_keep: 0,
        independency_factor: 0,
        n_sample_rate: 0,
        n_input_channels: 0,
        n_output_channels: 0,
        n_frame_time_slots: 0,
        n_qmf_bands: 0,
        n_hybrid_bands: 0,
        n_frame_length: 0,
        n_samples_next: 0,
        n_analysis_length_time_slots: 0,
        n_analysis_lookahead_time_slots: 0,
        n_update_hybrid_position_time_slots: 0,
        pn_output_bits: vec![0; MAX_BITSTREAM_DELAY as usize],
        n_input_delay: 0,
        n_output_buffer_delay: 0,
        n_surround_analysis_buffer_delay: 0,
        n_bitstream_delay_buffer: 0,
        n_bitstream_buffer_read: 0,
        n_bitstream_buffer_write: 0,
        n_discard_out_frames: 0,
        avoid_keep: 0,
        use_coarse_quant_cld: 0,
        use_coarse_quant_icc: 0,
        use_coarse_quant_cpc: 0,
        use_coarse_quant_arb_dmx: 0,
        quant_mode: 0,
        core_coder_delay: 0,
        time_alignment: 0,
        independency_count: 0,
        independency_flag: 0,
        pp_tr_curr_pos: vec![[0; MAX_NUM_TRANS]; max_ch_in],
        tr_prev_pos: [0; 2 * MAX_NUM_TRANS],
        frame_win_list: FrameWinList::default(),
        save_frame: SpatialFrame::default(),
        space_tree_setup: SpaceTreeSetup::default(),
        ssc_buf: Mpeg4SpaceEncSscBuf { n_ssc_size_bits: 0, p_ssc: vec![0; MAX_SSC_BYTES as usize] },
        p_frame_window_ana_fdk: core::array::from_fn(|_| {
            vec![0 as FixpWin; setup.max_analysis_length_time_slots as usize]
        }),
        // FilterStates is allocated here in C (the filter bank records only that it is set).
        ph_qmf_filt_in_fdk: vec![
            QmfFilterBank { filter_states: true, ..Default::default() };
            max_ch_in
        ],
        qmf_filter_states: vec![vec![0 as FixpSgl; 2 * 5 * max_qmf]; max_ch_in],
        ph_dc_filter_sig_in: dc,
        ph_onset: onset,
        h_space_tree,
        h_bitstream_formatter,
        h_static_gain_config,
        h_static_gain,
        h_delay,
        h_enhanced_time_dmx,
        pp_time_sig_in_fdk: vec![
            vec![
                0 as IntPcm;
                setup.max_frame_length as usize
                    + MAX_DELAY_SURROUND_ANALYSIS as usize
            ];
            max_ch_in
        ],
        pp_time_sig_delay_in_fdk: vec![
            vec![0 as IntPcm; MAX_DELAY_SURROUND_ANALYSIS as usize];
            max_ch_in
        ],
        // pppHybridIn__FDK buffer can be reduced by maxFrameTimeSlots/2 slots for SACENC_212
        // mode
        ppp_hybrid_in_fdk: hybrid(
            (setup.max_analysis_length_time_slots - (setup.max_frame_time_slots >> 1)) as usize,
        ),
        ppp_hybrid_in_static_fdk: hybrid(setup.max_hybrid_in_static_slots as usize),
        p_parameter_band2_hybrid_band_offset: vec![0; MAX_NUM_PARAM_BANDS],
        static_gain_scale: 0,
        p_encoder_input_ch_scale: vec![0; max_ch_in],
        static_time_domain_dmx_in_scale: vec![0; max_ch_in],
    });

    // save configuration used while encoder allocation.
    mp4_space_enc_init_default(&mut h_enc);

    *ph = Some(h_enc);
    SACENC_OK
}

/// `mp4SpaceEnc_InitDefault(hEnc)` (static in C)
fn mp4_space_enc_init_default(h_enc: &mut Mp4SpaceEncoder) -> FdkSacencError {
    // Get default static gain configuration.
    fdk_sacenc_static_gain_init_default_config(Some(&mut h_enc.h_static_gain_config))
}

/// `FDK_sacenc_configure(hEnc, hSetup)` (static in C)
fn fdk_sacenc_configure(h_enc: &mut Mp4SpaceEncoder, h_setup: &Mp4SpaceEncSetup) -> FdkSacencError {
    let mut error = SACENC_OK;

    h_enc.n_sample_rate = h_setup.sample_rate;
    h_enc.enc_mode = h_setup.enc_mode;
    h_enc.n_qmf_bands = mp4_space_enc_get_num_qmf_bands(h_enc.n_sample_rate) as u8;

    // Make sure that we have set time domain downmix for 212
    if h_setup.enc_mode == SACENC_212 && h_setup.b_time_domain_dmx == 0 {
        error = SACENC_INVALID_CONFIG;
    } else {
        h_enc.use_time_dom_downmix = h_setup.b_time_domain_dmx;
    }

    h_enc.time_alignment = h_setup.time_alignment;
    h_enc.quant_mode = h_setup.quant_mode;

    h_enc.use_coarse_quant_cld = h_setup.b_use_coarse_quant;
    h_enc.use_coarse_quant_cpc = h_setup.b_use_coarse_quant;
    h_enc.use_frame_keep = (h_setup.b_ld_mode == 2) as u8;
    h_enc.use_coarse_quant_icc = 0; // not available
    h_enc.use_coarse_quant_arb_dmx = 0; // not available for user right now
    h_enc.independency_factor = h_setup.independency_factor;
    h_enc.independency_count = 0;
    h_enc.independency_flag = 1;

    // set number of Hybrid bands
    h_enc.n_hybrid_bands = h_enc.n_qmf_bands;
    h_enc.n_frame_time_slots = h_setup.frame_time_slots as u8;
    mp4_space_enc_init_num_param_bands(h_enc, h_setup.n_param_bands);

    error
}

/// `FDK_sacenc_init(hMp4SpaceEnc, dmxDelay)`: finalizes the opening of the encoder with the
/// downmix (core coder) delay.
pub fn fdk_sacenc_init(h_enc: Option<&mut Mp4SpaceEncoder>, dmx_delay: i32) -> FdkSacencError {
    let Some(h_enc) = h_enc else {
        return SACENC_INVALID_HANDLE;
    };
    let init_states_flag = 1;

    let user = h_enc.user;
    let error = fdk_sacenc_configure(h_enc, &user);
    if error != SACENC_OK {
        return error;
    }

    h_enc.b_enc_mode_212_only = h_enc.setup.b_enc_mode_212;

    // Slots per Frame and Frame Length
    if h_enc.n_frame_time_slots < 1 {
        return SACENC_INVALID_CONFIG;
    }
    h_enc.n_frame_length = h_enc.n_qmf_bands as u32 * h_enc.n_frame_time_slots as u32;

    let nfts = h_enc.n_frame_time_slots as i32;
    if h_enc.use_frame_keep == 1 {
        h_enc.n_analysis_length_time_slots = 3 * nfts;
        h_enc.n_update_hybrid_position_time_slots = nfts;
    } else {
        h_enc.n_analysis_length_time_slots = 2 * nfts;
        h_enc.n_update_hybrid_position_time_slots = 0;
    }

    h_enc.n_analysis_lookahead_time_slots = h_enc.n_analysis_length_time_slots - 3 * nfts / 2;

    // init parameterBand2hybridBandOffset table
    fdk_sacenc_calc_parameter_band2_hybrid_band_offset(
        h_enc.n_param_bands as BoxSubbandConfig,
        h_enc.n_hybrid_bands as i32,
        &mut h_enc.p_parameter_band2_hybrid_band_offset,
    );

    // Fill Setup structure for Space Tree
    let mut space_tree_setup = h_enc.space_tree_setup;
    let error = mp4_space_enc_fill_space_tree_setup(h_enc, &mut space_tree_setup);
    h_enc.space_tree_setup = space_tree_setup;
    if error != SACENC_OK {
        return error;
    }

    // Init space tree configuration
    let error = fdk_sacenc_space_tree_init(
        &mut h_enc.h_space_tree,
        &h_enc.space_tree_setup,
        true,
        h_enc.use_frame_keep as i32,
    );
    if error != SACENC_OK {
        return error;
    }

    // Get space tree description and resulting number of input/output channels
    {
        let mut space_tree_description = SpaceTreeDescription::default();
        let error =
            fdk_sacenc_space_tree_get_description(&h_enc.h_space_tree, &mut space_tree_description);
        if error != SACENC_OK {
            return error;
        }

        // space tree description describes decoder configuration
        h_enc.n_input_channels = space_tree_description.n_out_channels;
        h_enc.n_output_channels = space_tree_description.n_in_channels;
    }

    let n_ch_in_arb_dmx = 0;
    let n_in = (h_enc.n_input_channels as usize).min(h_enc.setup.max_ch_in as usize);

    // INITIALIZATION
    for ch in 0..n_in {
        // scaling in analysis qmf filterbank (7)
        h_enc.p_encoder_input_ch_scale[ch] = 7;
        // additional scaling in qmf prototype filter for low delay
        h_enc.p_encoder_input_ch_scale[ch] += 1;
        h_enc.p_encoder_input_ch_scale[ch] += DC_FILTER_SF;
    }

    // Init analysis filterbank
    for ch in 0..n_in {
        let qmf = &mut h_enc.ph_qmf_filt_in_fdk[ch];
        qmf.flags = update_qmf_flags(qmf.flags, (init_states_flag == 0) as i32);
        let flags = qmf.flags;
        let nq = h_enc.n_qmf_bands as i32;
        if 0 != qmf_init_analysis_filter_bank(
            qmf,
            Some(&mut h_enc.qmf_filter_states[ch][..]),
            1,
            nq,
            nq,
            nq,
            flags,
        ) {
            return SACENC_INIT_ERROR;
        }
    }

    // Initialize DC Filter.
    for ch in 0..n_in {
        let error =
            fdk_sacenc_init_dc_filter(&mut h_enc.ph_dc_filter_sig_in[ch], h_enc.n_sample_rate);
        if error != SACENC_OK {
            return error;
        }
    }

    // Init onset detect.
    {
        // init onset detect configuration struct
        let onset_detect_config = OnsetDetectConfig {
            max_time_slots: nfts,
            lower_bound_onset_detection: freq2_hybrid_band(
                1725,
                h_enc.n_sample_rate,
                h_enc.n_qmf_bands as u32,
            ),
            upper_bound_onset_detection: h_enc.n_hybrid_bands as i32,
        };

        for ch in 0..n_in {
            let error =
                fdk_sacenc_onset_detect_init(&mut h_enc.ph_onset[ch], &onset_detect_config, 1);
            if error != SACENC_OK {
                return error;
            }
        }
    }

    {
        // init windowing
        let framewindow_config =
            FrameWindowConfig { n_time_slots_max: nfts, b_frame_keep: h_enc.use_frame_keep as i32 };

        let error = fdk_sacenc_frame_window_init(&mut h_enc.h_frame_window, &framewindow_config);
        if error != SACENC_OK {
            return error;
        }
    }

    // Set encoder mode for static gain initialization.
    let error =
        fdk_sacenc_static_gain_set_enc_mode(Some(&mut h_enc.h_static_gain_config), h_enc.enc_mode);
    if error != SACENC_OK {
        return error;
    }

    // Init static gain.
    let error = fdk_sacenc_static_gain_init(
        &mut h_enc.h_static_gain,
        &h_enc.h_static_gain_config,
        &mut h_enc.static_gain_scale,
    );
    if error != SACENC_OK {
        return error;
    }

    for ch in 0..n_in {
        h_enc.p_encoder_input_ch_scale[ch] =
            h_enc.p_encoder_input_ch_scale[ch].wrapping_add(h_enc.static_gain_scale);
    }

    // enhanced downmix for stereo input
    if h_enc.use_time_dom_downmix != 0 {
        let pre_gain = *fdk_sacenc_get_pre_gain_ptr_fdk(&h_enc.h_static_gain);
        let error = fdk_sacenc_init_enhanced_time_domain_dmx(
            &mut h_enc.h_enhanced_time_dmx,
            &pre_gain,
            h_enc.static_gain_scale,
            fdk_sacenc_get_post_gain_fdk(&h_enc.h_static_gain),
            h_enc.static_gain_scale,
            h_enc.n_frame_length as i32,
        );
        if error != SACENC_OK {
            return error;
        }
    }

    // Create config structure for bitstream formatter including arbitrary downmix residual
    let error = fdk_sacenc_init_spatial_bitstream_encoder(Some(&mut h_enc.h_bitstream_formatter));
    if error != SACENC_OK {
        return error;
    }

    let mut ssc = h_enc.h_bitstream_formatter.spatial_specific_config;
    let error = fill_spatial_specific_config(h_enc, &mut ssc);
    h_enc.h_bitstream_formatter.spatial_specific_config = ssc;
    if error != SACENC_OK {
        return error;
    }

    let error = fdk_sacenc_write_spatial_specific_config(
        fdk_sacenc_get_spatial_specific_config(&mut h_enc.h_bitstream_formatter),
        &mut h_enc.ssc_buf.p_ssc,
        MAX_SSC_BYTES,
        &mut h_enc.ssc_buf.n_ssc_size_bits,
    );
    if error != SACENC_OK {
        return error;
    }

    // init delay compensation with dmx core coder delay; if no core coder is used, many other
    // buffers are initialized nevertheless
    let error = mp4_space_enc_init_delay_compensation(h_enc, dmx_delay);
    if error != SACENC_OK {
        return error;
    }

    // How much input do we need?
    h_enc.n_samples_next =
        (h_enc.n_frame_length as i32).wrapping_mul(h_enc.n_input_channels as i32 + n_ch_in_arb_dmx);
    h_enc.n_samples_valid = 0;

    SACENC_OK
}

/// `getAnalysisLengthTimeSlots(pFrameWindowAna, nTimeSlots)` (static in C): one past the last
/// non-zero window value.
pub fn get_analysis_length_time_slots(p_frame_window_ana: &[FixpWin], n_time_slots: i32) -> i32 {
    let mut i = n_time_slots - 1;
    while i >= 0 {
        if p_frame_window_ana[i as usize] != 0 {
            break;
        }
        i -= 1;
    }
    i + 1
}

/// `getAnalysisStartTimeSlot(pFrameWindowAna, nTimeSlots)` (static in C): the first non-zero
/// window value.
pub fn get_analysis_start_time_slot(p_frame_window_ana: &[FixpWin], n_time_slots: i32) -> i32 {
    let mut i = 0;
    while i < n_time_slots {
        if p_frame_window_ana[i as usize] != 0 {
            break;
        }
        i += 1;
    }
    i
}

/// Copies `n` values from `src[s..]` to `dst[d..]`, as far as both slices reach (see the
/// deviations in the [module documentation](self)).
fn copy_clipped<T: Copy>(dst: &mut [T], d: i32, src: &[T], s: i32, n: i32) {
    if n <= 0 || d < 0 || s < 0 {
        return;
    }
    let (d, s) = (d as usize, s as usize);
    let n = (n as usize).min(dst.len().saturating_sub(d)).min(src.len().saturating_sub(s));
    dst[d..d + n].copy_from_slice(&src[s..s + n]);
}

/// `__FeedDeinterPreScale(hEnc, pSamples, pOutputSamples, nSamples, isInputInterleaved,
/// inputBufferSizePerChannel, pnSamplesFed)` (static in C): copies the input into the time
/// signal buffers and computes the time domain downmix.
///
/// # Safety
/// See [`fdk_sacenc_encode`].
unsafe fn feed_deinter_pre_scale(
    h_enc: &mut Mp4SpaceEncoder,
    p_samples: *const IntPcm,
    p_output_samples: *mut IntPcm,
    n_samples: i32,
    is_input_interleaved: u32,
    input_buffer_size_per_channel: u32,
    pn_samples_fed: &mut u32,
) -> FdkSacencError {
    if p_samples.is_null() {
        return SACENC_INVALID_HANDLE;
    }
    if n_samples == 0 {
        return SACENC_INVALID_CONFIG; // Flushing not implemented
    }
    let n_ch_in = h_enc.n_input_channels as i32;
    let n_ch_in_with_dmx = n_ch_in;
    let samples_to_feed =
        fix_min(n_samples, h_enc.n_samples_next.wrapping_sub(h_enc.n_samples_valid));
    let n_samples_per_channel = div_i(samples_to_feed, n_ch_in_with_dmx);

    if (samples_to_feed < 0)
        || (rem_i(samples_to_feed, n_ch_in_with_dmx) != 0)
        || (samples_to_feed > n_ch_in_with_dmx.wrapping_mul(h_enc.n_frame_length as i32))
    {
        return SACENC_INVALID_CONFIG;
    }

    // no dmx align = default
    let n_input_delay = h_enc.n_input_delay;
    let p_input2_off = (n_input_delay as isize).wrapping_mul(n_ch_in_with_dmx as isize);

    for i in 0..(h_enc.n_input_channels as usize).min(h_enc.static_time_domain_dmx_in_scale.len()) {
        h_enc.static_time_domain_dmx_in_scale[i] = h_enc.static_gain_scale;
    }

    let n_sur = h_enc.n_surround_analysis_buffer_delay;
    let n_ch = (n_ch_in.max(0) as usize).min(h_enc.pp_time_sig_in_fdk.len());
    // N-channel-input
    for ch in 0..n_ch {
        let tin = &mut h_enc.pp_time_sig_in_fdk[ch];
        let tdel = &mut h_enc.pp_time_sig_delay_in_fdk[ch];
        // Write delayed time signal into time signal buffer
        copy_clipped(tin, 0, tdel, 0, n_sur);

        let read = |idx: isize| -> IntPcm {
            // SAFETY: the caller guarantees the input samples (see fdk_sacenc_encode).
            unsafe { *p_samples.offset(idx) }
        };
        let mut put = |pos: i32, v: IntPcm| {
            if let Some(x) = usize::try_from(pos).ok().and_then(|p| tin.get_mut(p)) {
                *x = v;
            }
        };

        if is_input_interleaved != 0 {
            // Add the new frame de-interleaved. Apply nSurroundAnalysisBufferDelay.
            for i in 0..n_input_delay.max(0) {
                put(n_sur + i, read(ch as isize + i as isize * n_ch_in_with_dmx as isize));
            }
            for i in 0..(n_samples_per_channel - n_input_delay).max(0) {
                put(
                    n_sur + n_input_delay + i,
                    read(p_input2_off + ch as isize + i as isize * n_ch_in_with_dmx as isize),
                );
            }
        } else {
            // Input is already deinterleaved, just copy
            let base = ch as isize * input_buffer_size_per_channel as isize;
            for i in 0..n_input_delay.max(0) {
                put(n_sur + i, read(base + i as isize));
            }
            for i in 0..(n_samples_per_channel - n_input_delay).max(0) {
                put(n_sur + n_input_delay + i, read(p_input2_off + base + i as isize));
            }
        }

        // Update time signal delay buffer
        copy_clipped(tdel, 0, tin, h_enc.n_frame_length as i32, n_sur);
    }

    // Crude TD Dmx: Time DomainDownmix + NO Arbitrary Downmix, Delay Added at pOutputBuffer
    if h_enc.use_time_dom_downmix > 0 {
        if (h_enc.use_time_dom_downmix == 1) || (h_enc.n_input_channels != 2) {
            return SACENC_INVALID_CONFIG;
        }
        // enhanced time domain downmix (for stereo input)
        if h_enc.enc_mode != SACENC_212 {
            // ppTimeSigOut__FDK is not allocated (see the module documentation).
            return SACENC_INVALID_HANDLE;
        }
        if p_output_samples.is_null() {
            return SACENC_INVALID_HANDLE;
        }

        let n = h_enc.h_enhanced_time_dmx.framelength.max(0) as usize;
        let mut out = vec![0 as IntPcm; n];
        fdk_sacenc_apply_enhanced_time_domain_dmx(
            &mut h_enc.h_enhanced_time_dmx,
            &[&h_enc.pp_time_sig_in_fdk[0][..], &h_enc.pp_time_sig_in_fdk[1][..]],
            &mut out,
            n_sur,
        );
        // SAFETY: the caller guarantees the output samples (see fdk_sacenc_encode).
        core::ptr::copy_nonoverlapping(out.as_ptr(), p_output_samples, n);
    }

    // update number of samples still to process
    h_enc.n_samples_valid = h_enc.n_samples_valid.wrapping_add(samples_to_feed);

    // return number of fed samples
    *pn_samples_fed = samples_to_feed as u32;

    SACENC_OK
}

/// `FDK_sacenc_encode(hMp4SpaceEnc, inBufDesc, outBufDesc, inargs, outargs)`: encodes one
/// frame.
///
/// # Safety
/// - The arrays of both descriptors must be valid for `num_bufs` entries, and `out_buf_desc`
///   must have at least 2 entries (`ppBase[1]` is the bitstream buffer).
/// - The input PCM buffer (type `FDK_BUF_TYPE_INPUT | FDK_BUF_TYPE_PCM_DATA`) must be valid for
///   reads of the samples the encoder reads: `nInputSamples` interleaved samples, or (not
///   interleaved) `inputBufferSizePerChannel * (nChannels - 1) + nInputSamples / nChannels`
///   samples.
/// - The output PCM buffer (type `FDK_BUF_TYPE_OUTPUT | FDK_BUF_TYPE_PCM_DATA`) must be valid
///   for writes of `nFrameLength` samples. It may be the input buffer.
/// - `out_buf_desc.pp_base[1]` must be `NULL` or valid for writes of
///   `pBufSize[1] * pEleSize[1]` bytes and must not overlap the PCM buffers.
pub unsafe fn fdk_sacenc_encode(
    h_mp4_space_enc: Option<&mut Mp4SpaceEncoder>,
    in_buf_desc: &FdkBufDescr,
    out_buf_desc: &FdkBufDescr,
    inargs: &SacencInArgs,
    outargs: &mut SacencOutArgs,
) -> FdkSacencError {
    let base = |d: &FdkBufDescr, idx: i32| -> *mut c_void {
        if idx < 0 {
            core::ptr::null_mut() // no defined reference result (see the module documentation)
        } else {
            // SAFETY: idx < num_bufs.
            unsafe { *d.pp_base.add(idx as usize) }
        }
    };
    let elems = |d: &FdkBufDescr, idx: i32| -> u32 {
        if idx < 0 {
            0
        } else {
            // SAFETY: idx < num_bufs.
            unsafe { div_u(*d.p_buf_size.add(idx as usize), *d.p_ele_size.add(idx as usize)) }
        }
    };

    let in_idx = get_buf_desc_idx(in_buf_desc, FDK_BUF_TYPE_INPUT | FDK_BUF_TYPE_PCM_DATA);
    let p_input_samples = base(in_buf_desc, in_idx) as *const IntPcm;

    let out_idx = get_buf_desc_idx(out_buf_desc, FDK_BUF_TYPE_OUTPUT | FDK_BUF_TYPE_PCM_DATA);
    let p_output_samples = base(out_buf_desc, out_idx) as *mut IntPcm;
    let n_output_samples_buffer_size = elems(out_buf_desc, out_idx) as i32;

    let Some(h_enc) = h_mp4_space_enc else {
        return SACENC_INVALID_HANDLE;
    };
    if p_input_samples.is_null() {
        return SACENC_INVALID_HANDLE;
    }

    let mut curr_trans_pos: i32 = -1;

    // Improve Code Readability
    let n_ch_in = h_enc.n_input_channels as i32;
    let n_ch_in_with_dmx = n_ch_in;
    let n_ch_out = h_enc.n_output_channels as i32;
    let n_samples_per_channel = div_i(inargs.n_input_samples, n_ch_in_with_dmx);
    let n_output_samples_max = n_samples_per_channel.wrapping_mul(n_ch_out);
    let n_frame_time_slots = h_enc.n_frame_time_slots as i32;

    let mut encoder_input_ch_scale = [0i32; SACENC_MAX_INPUT_CHANNELS];
    let mut n_frame_time_slots_reduction = 0;

    if h_enc.enc_mode == SACENC_212 {
        n_frame_time_slots_reduction = n_frame_time_slots >> 1;
    }

    let n_ch = (n_ch_in.max(0) as usize).min(SACENC_MAX_INPUT_CHANNELS);
    encoder_input_ch_scale[..n_ch].copy_from_slice(&h_enc.p_encoder_input_ch_scale[..n_ch]);

    // Sanity Check
    if 0 != rem_i(inargs.n_input_samples, n_ch_in_with_dmx) {
        return SACENC_INVALID_CONFIG;
    }

    // Get Frame Data Handle: the one spatial frame of the bitstream formatter.

    // Independent Frames Counters
    if h_enc.n_discard_out_frames > 0 {
        // Independent Frames if they should be discarded, Reset Counter
        h_enc.independency_count = 0; // Reset the counter, first valid frame is an independent one
        h_enc.independency_flag = 1;
    } else {
        h_enc.independency_flag = (h_enc.independency_count == 0) as i32;
        if h_enc.independency_factor > 0 {
            h_enc.independency_count = h_enc.independency_count.wrapping_add(1);
            h_enc.independency_count =
                rem_i(h_enc.independency_count, h_enc.independency_factor as i32);
        } else {
            // independencyFactor == 0
            h_enc.independency_count = -1;
        }
    }

    // Time signal preprocessing: Feed, Deinterleave, Pre-Scale the input time signals
    let error = feed_deinter_pre_scale(
        h_enc,
        p_input_samples,
        p_output_samples,
        inargs.n_input_samples,
        inargs.is_input_interleaved,
        inargs.input_buffer_size_per_channel,
        &mut outargs.n_samples_consumed,
    );
    if error != SACENC_OK {
        return error;
    }

    if h_enc.n_samples_next != h_enc.n_samples_valid {
        return SACENC_INVALID_CONFIG;
    }

    let is_212_only = h_enc.enc_mode == SACENC_212 && h_enc.b_enc_mode_212_only != 0;
    let n_hybrid_bands = h_enc.n_hybrid_bands as i32;
    let n_update = h_enc.n_update_hybrid_position_time_slots;

    if is_212_only {
        for ch in 0..n_ch {
            for slot in 0..n_frame_time_slots {
                let s = n_update + n_frame_time_slots - n_frame_time_slots_reduction + slot;
                set_cplx_vec(&mut h_enc.ppp_hybrid_in_fdk[ch][s as usize], 0, n_hybrid_bands);
            }
        }
    }

    // Time / Frequency: T/F audio input channels
    let n_qmf = h_enc.n_qmf_bands as usize;
    let lookahead = h_enc.n_analysis_lookahead_time_slots;
    for ch in 0..n_ch {
        let mut p_qmf_in_real = [0 as FixpDbl; MAX_QMF_BANDS];
        let mut p_qmf_in_imag = [0 as FixpDbl; MAX_QMF_BANDS];
        let p_pre_gain = *fdk_sacenc_get_pre_gain_ptr_fdk(&h_enc.h_static_gain);

        for ts in 0..n_frame_time_slots.max(0) as usize {
            let p_time_in = &mut h_enc.pp_time_sig_in_fdk[ch][ts * n_qmf..];

            // Apply DC filter on input channels
            let error = fdk_sacenc_apply_dc_filter_in_place(
                &mut h_enc.ph_dc_filter_sig_in[ch],
                p_time_in,
                n_qmf as i32,
            );
            if error != SACENC_OK {
                return error;
            }

            // QMF filterbank
            let mut p_work_buffer = [0 as FixpDbl; MAX_QMF_BANDS << 1];

            qmf_analysis_filtering_slot::<IntPcm>(
                &h_enc.ph_qmf_filt_in_fdk[ch],
                &mut h_enc.qmf_filter_states[ch],
                &mut p_qmf_in_real,
                &mut p_qmf_in_imag,
                p_time_in,
                1,
                &mut p_work_buffer,
            );

            // Apply pre-scale after filterbank
            let dst = &mut h_enc.ppp_hybrid_in_fdk[ch][(lookahead + ts as i32) as usize];
            let nb = n_hybrid_bands.max(0) as usize;
            if MAXVAL_GAIN != p_pre_gain[ch] {
                for i in 0..nb {
                    dst[i].re = f_mult(p_qmf_in_real[i], p_pre_gain[ch]);
                    dst[i].im = f_mult(p_qmf_in_imag[i], p_pre_gain[ch]);
                }
            } else {
                for i in 0..nb {
                    dst[i].re = p_qmf_in_real[i];
                    dst[i].im = p_qmf_in_imag[i];
                }
            }
        }
    }

    if is_212_only {
        let n = n_update + n_frame_time_slots - n_frame_time_slots_reduction;
        for ch in 0..n_ch {
            for slot in 0..n.max(0) as usize {
                copy_cplx_vec(
                    &mut h_enc.ppp_hybrid_in_fdk[ch][slot],
                    &h_enc.ppp_hybrid_in_static_fdk[ch][slot],
                    n_hybrid_bands,
                );
            }
        }
        for ch in 0..n_ch {
            for slot in 0..n.max(0) as usize {
                let src = &h_enc.ppp_hybrid_in_fdk[ch][n_frame_time_slots as usize + slot];
                copy_cplx_vec(&mut h_enc.ppp_hybrid_in_static_fdk[ch][slot], src, n_hybrid_bands);
            }
        }
    }

    // Onset Detection: detection of transients, build framing
    for ch in 0..n_ch {
        if ch != 3 {
            // !LFE
            let mut pos = h_enc.pp_tr_curr_pos[ch];
            let error = fdk_sacenc_onset_detect_apply(
                &mut h_enc.ph_onset[ch],
                n_frame_time_slots,
                n_hybrid_bands,
                &h_enc.ppp_hybrid_in_fdk[ch][lookahead.max(0) as usize..],
                encoder_input_ch_scale[ch],
                h_enc.tr_prev_pos[1], // contains previous Transient
                &mut pos,
            );
            h_enc.pp_tr_curr_pos[ch] = pos;
            if error != SACENC_OK {
                return error;
            }

            if h_enc.use_frame_keep == 0 {
                h_enc.pp_tr_curr_pos[ch][0] = -1;
            }

            // Find first Transient Position
            if (h_enc.pp_tr_curr_pos[ch][0] >= 0)
                && ((curr_trans_pos < 0) || (h_enc.pp_tr_curr_pos[ch][0] < curr_trans_pos))
            {
                curr_trans_pos = h_enc.pp_tr_curr_pos[ch][0];
            }
        }
    }

    if h_enc.use_frame_keep == 1 && ((curr_trans_pos != -1) || (h_enc.independency_flag == 1)) {
        h_enc.avoid_keep = NUM_KEEP_WINDOWS;
        curr_trans_pos = -1;
    }

    // Save previous Transient Position
    h_enc.tr_prev_pos[0] = fix_max(-1, h_enc.tr_prev_pos[1].wrapping_sub(n_frame_time_slots));
    h_enc.tr_prev_pos[1] = curr_trans_pos;

    // Update Onset Detection Energy Buffer
    for ch in 0..n_ch {
        let error = fdk_sacenc_onset_detect_update(&mut h_enc.ph_onset[ch], n_frame_time_slots);
        if error != SACENC_OK {
            return error;
        }
    }

    // Framing
    let tr_prev_pos = h_enc.tr_prev_pos;
    let error = fdk_sacenc_frame_window_get_window(
        &h_enc.h_frame_window,
        &tr_prev_pos,
        n_frame_time_slots,
        &mut h_enc.h_bitstream_formatter.frame.framing_info,
        &mut h_enc.p_frame_window_ana_fdk,
        &mut h_enc.frame_win_list,
        h_enc.avoid_keep,
    );
    if error != SACENC_OK {
        return error;
    }

    // MPS Processing:
    let mut win_cnt = 0usize;
    for ps in 0..h_enc.frame_win_list.n.max(0) as usize {
        let p_frame_data = &mut h_enc.h_bitstream_formatter.frame;
        // Analysis Windowing
        if h_enc.frame_win_list.dat[ps].hold == FW_HOLD {
            // ONLY COPY AND HOLD PREVIOUS PARAMETERS
            let error =
                fdk_sacenc_duplicate_parameter_set(&h_enc.save_frame, 0, p_frame_data, ps as i32);
            if error != SACENC_OK {
                return error;
            }
        } else {
            // NEW WINDOW
            let win = &h_enc.p_frame_window_ana_fdk[win_cnt];
            let n_analysis_length_time_slots =
                get_analysis_length_time_slots(win, h_enc.n_analysis_length_time_slots);
            let analysis_start_time_slot =
                get_analysis_start_time_slot(win, h_enc.n_analysis_length_time_slots);

            // perform main signal analysis windowing in fdk_sacenc_spaceTree_Apply();
            // pppProcDataIn__FDK = pppHybridIn__FDK in 212 mode
            if !is_212_only {
                // pppProcDataIn__FDK is not allocated (see the module documentation).
                return SACENC_INVALID_HANDLE;
            }

            let error = fdk_sacenc_space_tree_apply(
                &mut h_enc.h_space_tree,
                &h_enc.p_parameter_band2_hybrid_band_offset,
                ps as i32,
                n_ch_in,
                n_analysis_length_time_slots,
                analysis_start_time_slot,
                n_hybrid_bands,
                win,
                &mut h_enc.ppp_hybrid_in_fdk,
                p_frame_data,
                h_enc.avoid_keep,
                &encoder_input_ch_scale,
            );
            if error != SACENC_OK {
                return error;
            }

            // Save spatial frame for potential hold parameter set
            let error = fdk_sacenc_duplicate_parameter_set(
                p_frame_data,
                ps as i32,
                &mut h_enc.save_frame,
                0,
            );
            if error != SACENC_OK {
                return error;
            }

            win_cnt += 1;
        }
        if h_enc.avoid_keep > 0 {
            h_enc.avoid_keep -= 1;
        }
    }
    // ---- End of Processing Loop ----

    // Update hybridInReal/Imag buffer: only without 212 mode (see the module documentation).

    // Spatial Tonality: Smooth config off.
    h_enc.h_bitstream_formatter.frame.smg_data = SmgData::default();

    // Create bitstream: control independecy flag, write spatial frame, return bitstream
    if h_enc.enc_mode != SACENC_212 {
        // ppBitstreamDelayBuffer is not allocated (see the module documentation).
        return SACENC_INVALID_HANDLE;
    }
    // no bitstream delay buffer for SACENC_212 mode, write bitstream directly into the
    // sacOutBuffer buffer which is provided by the core routine
    let bs_ptr = if out_buf_desc.num_bufs > 1 {
        base(out_buf_desc, 1) as *mut u8
    } else {
        core::ptr::null_mut()
    };
    if bs_ptr.is_null() {
        return SACENC_INVALID_HANDLE;
    }
    let bs_len = if out_buf_desc.num_bufs > 1 {
        (*out_buf_desc.p_buf_size.add(1)).wrapping_mul(*out_buf_desc.p_ele_size.add(1)) as usize
    } else {
        0
    };
    // SAFETY: the caller guarantees the bitstream buffer.
    let p_bitstream_delay_buffer = core::slice::from_raw_parts_mut(bs_ptr, bs_len);

    h_enc.h_bitstream_formatter.frame.bs_independency_flag = h_enc.independency_flag as u8;

    // `&pnOutputBits[nBitstreamBufferWrite]`, which the writer leaves unchanged on an early
    // error (where the reference has no defined result, the write is dropped).
    let write = usize::try_from(h_enc.n_bitstream_buffer_write)
        .ok()
        .filter(|&w| w < h_enc.pn_output_bits.len());
    let mut n_bits = write.map_or(0, |w| h_enc.pn_output_bits[w]);
    let error = fdk_sacenc_write_spatial_frame(
        p_bitstream_delay_buffer,
        MAX_MPEGS_BYTES,
        &mut n_bits,
        &mut h_enc.h_bitstream_formatter,
    );
    if let Some(w) = write {
        h_enc.pn_output_bits[w] = n_bits;
    }
    if error != SACENC_OK {
        return error;
    }

    // return bitstream info
    let bs_idx = get_buf_desc_idx(out_buf_desc, FDK_BUF_TYPE_OUTPUT | FDK_BUF_TYPE_BS_DATA);
    if (h_enc.n_discard_out_frames == 0) && (bs_idx != -1) {
        let out_bits = usize::try_from(h_enc.n_bitstream_buffer_read)
            .ok()
            .and_then(|r| h_enc.pn_output_bits.get(r))
            .copied()
            .unwrap_or(0);

        if (out_bits.wrapping_add(7) / 8) > elems(out_buf_desc, bs_idx) as i32 {
            outargs.n_output_bits = 0;
            return SACENC_ENCODE_ERROR;
        }

        // return number of valid bits
        outargs.n_output_bits = out_bits;
    } else {
        // No spatial data should be returned if the current frame is to be discarded.
        outargs.n_output_bits = 0;
    }

    // update pointers
    h_enc.n_bitstream_buffer_read =
        rem_i(h_enc.n_bitstream_buffer_read.wrapping_add(1), h_enc.n_bitstream_delay_buffer);
    h_enc.n_bitstream_buffer_write =
        rem_i(h_enc.n_bitstream_buffer_write.wrapping_add(1), h_enc.n_bitstream_delay_buffer);

    // Set Output Parameters: don't output samples in case frames to be discarded
    let n_output_samples = if h_enc.n_discard_out_frames == 0 { n_output_samples_max } else { 0 };
    if n_output_samples > n_output_samples_buffer_size {
        return SACENC_INVALID_CONFIG;
    }
    outargs.n_output_samples = n_output_samples;

    // !bQmfOutput: no output delay in SACENC_212 mode
    if h_enc.use_time_dom_downmix == 0 {
        if p_output_samples.is_null() {
            // Guarded (212 mode requires the time domain downmix).
            return SACENC_INVALID_HANDLE;
        }
        let n = n_output_samples_max.max(0) as usize;
        // SAFETY: the caller guarantees the output samples.
        let out = core::slice::from_raw_parts_mut(p_output_samples, n);
        let error = fdk_sacenc_static_post_gain_apply_fdk(
            &h_enc.h_static_gain,
            out,
            n_output_samples_max,
            0,
        );
        if error != SACENC_OK {
            return error;
        }
    }

    if h_enc.n_discard_out_frames > 0 {
        h_enc.n_discard_out_frames -= 1;
    }

    // Invalidate Input Buffer
    h_enc.n_samples_valid = 0;

    SACENC_OK
}

/// `FDK_sacenc_close(phMp4SpaceEnc)`: closes the encoder and frees its memory.
pub fn fdk_sacenc_close(ph_mp4_space_enc: Option<&mut HandleMp4SpaceEncoder>) -> FdkSacencError {
    if let Some(h) = ph_mp4_space_enc {
        *h = None;
    }
    SACENC_OK
}

/// `mp4SpaceEnc_InitDelayCompensation(hMp4SpaceEnc, coreCoderDelay)` (static in C)
fn mp4_space_enc_init_delay_compensation(
    h_enc: &mut Mp4SpaceEncoder,
    core_coder_delay: i32,
) -> FdkSacencError {
    h_enc.core_coder_delay = core_coder_delay;

    let error = fdk_sacenc_delay_init(
        &mut h_enc.h_delay,
        h_enc.n_qmf_bands as i32,
        h_enc.n_frame_length as i32,
        core_coder_delay,
        h_enc.time_alignment,
    );
    if error != SACENC_OK {
        return error;
    }

    fdk_sacenc_delay_set_dmx_align(&mut h_enc.h_delay, 0);
    fdk_sacenc_delay_set_time_dom_dmx(&mut h_enc.h_delay, (h_enc.use_time_dom_downmix >= 1) as i32);
    fdk_sacenc_delay_set_minimize_delay(&mut h_enc.h_delay, 1);

    let error = fdk_sacenc_delay_sub_calulate_buffer_delays(&mut h_enc.h_delay);
    if error != SACENC_OK {
        return error;
    }

    // init output delay compensation
    h_enc.n_bitstream_delay_buffer =
        fdk_sacenc_delay_get_bitstream_frame_buffer_size(&h_enc.h_delay);
    h_enc.n_output_buffer_delay = fdk_sacenc_delay_get_output_audio_buffer_delay(&h_enc.h_delay);
    h_enc.n_surround_analysis_buffer_delay =
        fdk_sacenc_delay_get_surround_analysis_buffer_delay(&h_enc.h_delay);
    h_enc.n_bitstream_buffer_read = 0;
    h_enc.n_bitstream_buffer_write = h_enc.n_bitstream_delay_buffer.wrapping_sub(1);

    if h_enc.enc_mode == SACENC_212 {
        // mode 212 expects no bitstream delay
        if h_enc.n_bitstream_buffer_write != h_enc.n_bitstream_buffer_read {
            return SACENC_PARAM_ERROR;
        }

        // mode 212 expects no output buffer delay
        if h_enc.n_output_buffer_delay != 0 {
            return SACENC_PARAM_ERROR;
        }
    }

    // Input delay to obtain a net encoder delay that is a multiple of the used framelength to
    // ensure synchronization of framing in artistic down-mix with the corresponding spatial
    // data.
    h_enc.n_discard_out_frames = fdk_sacenc_delay_get_discard_out_frames(&h_enc.h_delay);
    h_enc.n_input_delay = fdk_sacenc_delay_get_dmx_align_buffer_delay(&h_enc.h_delay);

    // reset independency Flag counter
    h_enc.independency_count = 0;
    h_enc.independency_flag = 1;

    // write some parameters to bitstream: only with a bitstream delay, which 212 mode refuses
    // above (ppBitstreamDelayBuffer is not allocated in 212 mode).
    if h_enc.n_bitstream_delay_buffer > 1 {
        return SACENC_INVALID_HANDLE;
    }

    if (h_enc.n_input_delay > MAX_DELAY_INPUT)
        || (h_enc.n_output_buffer_delay > MAX_DELAY_OUTPUT)
        || (h_enc.n_surround_analysis_buffer_delay > MAX_DELAY_SURROUND_ANALYSIS)
        || (h_enc.n_bitstream_delay_buffer > MAX_BITSTREAM_DELAY)
    {
        return SACENC_INIT_ERROR;
    }

    SACENC_OK
}

/// `__mapQuantMode(quantMode)` (static in C)
pub fn map_quant_mode(quant_mode: Mp4SpaceEncQuantMode) -> QuantMode {
    match quant_mode {
        SACENC_QUANTMODE_FINE => QUANTMODE_FINE,
        SACENC_QUANTMODE_EBQ1 => QUANTMODE_EBQ1,
        SACENC_QUANTMODE_EBQ2 => QUANTMODE_EBQ2,
        _ => QUANTMODE_INVALID,
    }
}

/// `FillSpatialSpecificConfig(hEnc, hSsc)` (static in C)
fn fill_spatial_specific_config(
    h_enc: &Mp4SpaceEncoder,
    h_ssc: &mut SpatialSpecificConfig,
) -> FdkSacencError {
    let mut space_tree_description = SpaceTreeDescription::default();

    // Get tree description
    let error =
        fdk_sacenc_space_tree_get_description(&h_enc.h_space_tree, &mut space_tree_description);
    if error != SACENC_OK {
        return error;
    }

    // Fill SSC
    *h_ssc = SpatialSpecificConfig::default(); // reset

    h_ssc.num_bands = h_enc.space_tree_setup.n_param_bands as i32; // for bsFreqRes

    // Fill tree configuration
    h_ssc.tree_description.num_ott_boxes = space_tree_description.n_ott_boxes as i32;
    h_ssc.tree_description.num_in_chan = space_tree_description.n_in_channels as i32;
    h_ssc.tree_description.num_out_chan = space_tree_description.n_out_channels as i32;

    for c in h_ssc.ott_config.iter_mut() {
        c.bs_ott_bands = h_ssc.num_bands;
    }

    match h_enc.enc_mode {
        SACENC_212 => h_ssc.bs_tree_config = TREE_212,
        _ => return SACENC_INVALID_CONFIG,
    }

    h_ssc.bs_sampling_frequency = h_enc.n_sample_rate as i32; // for bsSamplingFrequencyIndex
    h_ssc.bs_frame_length = h_enc.n_frame_time_slots as i32 - 1;

    // map decorr type
    h_ssc.bs_decorr_config = mp4_space_enc_get_decorr_config(h_enc.enc_mode);
    if DECORR_INVALID == h_ssc.bs_decorr_config {
        return SACENC_INVALID_CONFIG;
    }

    // map quantMode
    h_ssc.bs_quant_mode = map_quant_mode(h_enc.quant_mode);
    if QUANTMODE_INVALID == h_ssc.bs_quant_mode {
        return SACENC_INVALID_CONFIG;
    }

    // Configure Gains
    h_ssc.bs_fixed_gain_dmx = fdk_sacenc_static_gain_get_dmx_gain(&h_enc.h_static_gain);
    h_ssc.bs_env_quant_mode = 0;

    SACENC_OK
}

/// `mp4SpaceEnc_FillSpaceTreeSetup(hEnc, hSpaceTreeSetup)` (static in C)
fn mp4_space_enc_fill_space_tree_setup(
    h_enc: &Mp4SpaceEncoder,
    h_space_tree_setup: &mut SpaceTreeSetup,
) -> FdkSacencError {
    // map quantMode
    let tmp_quantmode = map_quant_mode(h_enc.quant_mode);
    if QUANTMODE_INVALID == tmp_quantmode {
        return SACENC_INVALID_CONFIG;
    }

    h_space_tree_setup.n_param_bands = h_enc.n_param_bands as u8;
    h_space_tree_setup.b_use_coarse_quant_tto_cld = h_enc.use_coarse_quant_cld;
    h_space_tree_setup.b_use_coarse_quant_tto_icc = h_enc.use_coarse_quant_icc;
    h_space_tree_setup.quant_mode = tmp_quantmode;
    h_space_tree_setup.n_hybrid_bands_max = h_enc.n_hybrid_bands;

    match h_enc.enc_mode {
        SACENC_212 => {
            h_space_tree_setup.mode = SPACETREE_212;
            h_space_tree_setup.n_channels_in_max = 2;
        }
        _ => return SACENC_INVALID_CONFIG,
    }

    SACENC_OK
}

/// `FDK_sacenc_getInfo(hMp4SpaceEnc, pInfo)`: information on the produced bitstream.
pub fn fdk_sacenc_get_info<'a>(
    h_mp4_space_enc: Option<&'a Mp4SpaceEncoder>,
    p_info: Option<&mut Mp4SpaceEncInfo<'a>>,
) -> FdkSacencError {
    let (Some(h_enc), Some(p_info)) = (h_mp4_space_enc, p_info) else {
        return SACENC_INVALID_HANDLE;
    };
    p_info.n_sample_rate = h_enc.n_sample_rate as i32;
    p_info.n_samples_frame = h_enc.n_frame_length as i32;
    p_info.n_total_input_channels = h_enc.n_input_channels as i32;
    p_info.n_dmx_delay = fdk_sacenc_delay_get_info_dmx_delay(&h_enc.h_delay);
    p_info.n_codec_delay = fdk_sacenc_delay_get_info_codec_delay(&h_enc.h_delay);
    p_info.n_decoder_delay = fdk_sacenc_delay_get_info_decoder_delay(&h_enc.h_delay);
    p_info.n_payload_delay =
        fdk_sacenc_delay_get_bitstream_frame_buffer_size(&h_enc.h_delay).wrapping_sub(1);
    p_info.n_discard_out_frames = h_enc.n_discard_out_frames;

    p_info.p_ssc_buf = Some(&h_enc.ssc_buf);
    SACENC_OK
}

/// `FDK_sacenc_setParam(hMp4SpaceEnc, param, value)`: sets one encoder parameter.
pub fn fdk_sacenc_set_param(
    h_mp4_space_enc: Option<&mut Mp4SpaceEncoder>,
    param: SpaceEncParam,
    value: u32,
) -> FdkSacencError {
    let Some(h) = h_mp4_space_enc else {
        return SACENC_INVALID_HANDLE;
    };
    let mut error = SACENC_OK;

    match param {
        SACENC_LOWDELAY => {
            if !((value == 0) || (value == 1) || (value == 2)) {
                return SACENC_INVALID_CONFIG;
            }
            h.user.b_ld_mode = value as u8;
        }

        SACENC_ENC_MODE => match value as i32 {
            SACENC_212 => h.user.enc_mode = value as i32,
            _ => error = SACENC_INVALID_CONFIG,
        },

        SACENC_SAMPLERATE => {
            if ((value as i32) < 0) || ((value as i32) > h.setup.max_samplingrate) {
                return SACENC_INVALID_CONFIG;
            }
            h.user.sample_rate = value;
        }

        SACENC_FRAME_TIME_SLOTS => {
            if ((value as i32) < 0) || ((value as i32) > h.setup.max_frame_time_slots) {
                return SACENC_INVALID_CONFIG;
            }
            h.user.frame_time_slots = value;
        }

        SACENC_PARAM_BANDS => match value as i32 {
            SACENC_BANDS_4 | SACENC_BANDS_5 | SACENC_BANDS_7 | SACENC_BANDS_9 | SACENC_BANDS_12
            | SACENC_BANDS_15 | SACENC_BANDS_23 => h.user.n_param_bands = value as i32,
            _ => error = SACENC_INVALID_CONFIG,
        },

        SACENC_TIME_DOM_DMX => {
            if !((value == 0) || (value == 2)) {
                return SACENC_INVALID_CONFIG;
            }
            h.user.b_time_domain_dmx = value as u8;
        }

        SACENC_DMX_GAIN => {
            if value > 7 {
                return SACENC_INVALID_CONFIG;
            }
            error = fdk_sacenc_static_gain_set_dmx_gain(
                Some(&mut h.h_static_gain_config),
                value as i32,
            );
        }

        SACENC_COARSE_QUANT => {
            if !((value == 0) || (value == 1)) {
                return SACENC_INVALID_CONFIG;
            }
            h.user.b_use_coarse_quant = value as u8;
        }

        SACENC_QUANT_MODE => match value as i32 {
            SACENC_QUANTMODE_FINE | SACENC_QUANTMODE_EBQ1 | SACENC_QUANTMODE_EBQ2 => {
                h.user.quant_mode = value as i32
            }
            _ => error = SACENC_INVALID_CONFIG,
        },

        SACENC_TIME_ALIGNMENT => {
            if (value as i32) < -32768 || (value as i32) > 32767 {
                return SACENC_INVALID_CONFIG;
            }
            h.user.time_alignment = value as i32;
        }

        SACENC_INDEPENDENCY_COUNT => h.independency_count = value as i32,

        SACENC_INDEPENDENCY_FACTOR => h.user.independency_factor = value,

        _ => error = SACENC_UNSUPPORTED_PARAMETER,
    }
    error
}

/// `FDK_sacenc_getLibInfo(info)`: enters the library information into the next free entry of
/// `info` (after the FDK tools).
pub fn fdk_sacenc_get_lib_info(info: Option<&mut [LibInfo]>) -> FdkSacencError {
    let Some(info) = info else {
        return SACENC_INVALID_HANDLE;
    };

    fdk_tools_get_lib_info(Some(&mut *info));

    // search for next free tab
    let mut i = 0usize;
    while i < FDK_MODULE_LAST as usize {
        match info.get(i) {
            Some(e) if e.module_id == FDK_NONE => break,
            Some(_) => {}
            // C reads FDK_MODULE_LAST entries; a shorter slice ends the search
            None => return SACENC_INIT_ERROR,
        }
        i += 1;
    }
    if i == FDK_MODULE_LAST as usize {
        return SACENC_INIT_ERROR;
    }

    let e = &mut info[i];
    e.module_id = FDK_MPSENC;
    e.build_date = SACENC_LIB_BUILD_DATE;
    e.build_time = SACENC_LIB_BUILD_TIME;
    e.title = SACENC_LIB_TITLE;
    e.version = lib_version(SACENC_LIB_VL0, SACENC_LIB_VL1, SACENC_LIB_VL2);
    lib_version_string(e);

    // Capability flags
    e.flags = 0;

    SACENC_OK
}

/// `mp4SpaceEnc_GetDecorrConfig(encMode)` (static in C)
fn mp4_space_enc_get_decorr_config(enc_mode: Mp4SpaceEncMode) -> DecorrConfig {
    // set decorrConfig dependent on tree mode
    match enc_mode {
        SACENC_212 => DECORR_QMFSPLIT0,
        _ => DECORR_INVALID,
    }
}

/// `mp4SpaceEnc_InitNumParamBands(hEnc, nParamBands)` (static in C)
fn mp4_space_enc_init_num_param_bands(
    h_enc: &mut Mp4SpaceEncoder,
    n_param_bands: Mp4SpaceEncBandsConfig,
) -> FdkSacencError {
    // Set/Check nParamBands
    if P_VALID_BANDS_LD.contains(&(n_param_bands as u8)) {
        h_enc.n_param_bands = n_param_bands;
    } else {
        h_enc.n_param_bands = SACENC_BANDS_INVALID;
    }
    SACENC_OK
}
