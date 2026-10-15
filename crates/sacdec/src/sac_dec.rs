// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libSACdec/src/sac_dec.h, libSACdec/src/sac_dec.cpp.

//! Port of `sac_dec.h` and `sac_dec.cpp`: the structures of the spatial decoder and its open,
//! init and frame processing functions.
//!
//! # Memory
//!
//! `FDK_SpatialDecOpen()` allocates the arrays of `spatialDec` with the sizes of the only
//! supported configuration (`DECODER_LEVEL_0`: 1 input, 2 output channels, 64 QMF bands, 71
//! hybrid bands, 1 OTT box, 1 residual channel, 1 decorrelator). The port keeps them as fixed
//! size arrays of [`SpatialDec`]; the contiguous `FDK_ALLOCATE_MEMORY_2D/3D` matrices become
//! nested arrays (the `FDKmemclear` over a whole matrix in `SpatialDecApplyM2()` clears all
//! rows). The hybrid filter states and the decorrelator buffer are owned by the filter bank and
//! the decorrelator ([`fdk_aac_rs_fdk::hybrid`], [`fdk_aac_rs_fdk::decorrelate`]). The
//! sub-structures `smoothState`, `reshapeBBEnvState` and `hStpDec` are members.
//!
//! # Pointers
//!
//! - `pQmfDomain` (the QMF domain of the AAC decoder) is not kept: the functions that use it take
//!   it as a parameter (`qd`).
//! - `pConfigCurrent` points to `spatialSpecificConfig[bsFrameDecode]` of the MPEG Surround
//!   decoder (or is `NULL` before the first configuration): [`SpatialDec::p_config_current`]
//!   records whether it is set, and the functions that read or write it take the configuration
//!   as a parameter (`cfg`). `FDK_SpatialDecInit()` is always called with the same
//!   configuration in its `pSpatialSpecificConfig` parameter; the port uses the parameter.
//! - `timeOut__FDK` (the output buffer of the current frame) is a parameter of
//!   [`spatial_dec_apply_frame`].
//! - `kernels` refers to one of the `kernels_*` tables, `pActivM2ParamBands` is always `NULL`.
//! - The arrays of row pointers that the C functions take (`FIXP_DBL **`) are always the arrays
//!   of the decoder itself; the port's functions use them directly.
//! - The `LOSSLESSDATA::state` pointer of entry `i` of a lossless data array always points to
//!   entry `i` of the matching array of the [`BsLlState`]; the port passes that entry.

use fdk_aac_rs_fdk::common_fix::*;
use fdk_aac_rs_fdk::decorrelate::{
    fdk_decorrelate_close, fdk_decorrelate_init, fdk_decorrelate_open, DecorrDec,
    DECORR_BUFFER_LEN, DECORR_LD, DECORR_PS, DECORR_USAC, DUCKER_AUTOMATIC,
};
use fdk_aac_rs_fdk::fdk_assert;
use fdk_aac_rs_fdk::hybrid::*;
use fdk_aac_rs_fdk::qmf::*;
use fdk_aac_rs_fdk::qmf_domain::*;
use fdk_aac_rs_fdk::syslib_channel_map_descr::FdkChannelMapDescr;

use crate::sac_bitdec::*;
use crate::sac_calc_m1_and_m2::*;
use crate::sac_dec_conceal::*;
use crate::sac_dec_interface::*;
use crate::sac_process::*;
use crate::sac_qmf::*;
use crate::sac_reshape_bb_env::*;
use crate::sac_rom::*;
use crate::sac_smoothing::*;
use crate::sac_stp::*;
use crate::sac_tsd::*;

/// `ICCdefault`
pub const ICC_DEFAULT: i8 = 0;
/// `IPDdefault`
pub const IPD_DEFAULT: i8 = 0;
/// `arbdmxGainDefault`
pub const ARBDMX_GAIN_DEFAULT: i8 = 0;
/// `CPCdefault`
pub const CPC_DEFAULT: i8 = 10;
/// `tttCLD1default`
pub const TTT_CLD1_DEFAULT: i8 = 15;
/// `tttCLD2default`
pub const TTT_CLD2_DEFAULT: i8 = 0;

// `IS_HQ_ONLY(aot)` is not ported: it is never used and refers to `AOT_RSVD50`, which
// v2.0.3 does not define.

/// `PC_NUM_BANDS`
pub const PC_NUM_BANDS: i32 = 8;
/// `PC_NUM_HYB_BANDS`
pub const PC_NUM_HYB_BANDS: i32 = PC_NUM_BANDS - 3 + 10;

/// `MAX_HYBRID_BANDS`
pub const MAX_HYBRID_BANDS: usize = (MAX_NUM_QMF_BANDS - 3 + 10) as usize;
/// `HYBRID_FILTER_DELAY`
pub const HYBRID_FILTER_DELAY: i32 = 6;

/// `MAX_RESIDUAL_FRAMES`
pub const MAX_RESIDUAL_FRAMES: i32 = 4;
/// `MAX_RESIDUAL_BISTREAM`: 48000 bps * 3 res / (8 * 44100 / 2048)
pub const MAX_RESIDUAL_BISTREAM: i32 = 836;
/// `MAX_MDCT_COEFFS`
pub const MAX_MDCT_COEFFS: i32 = 1024;
/// `SACDEC_RESIDUAL_BS_BUF_SIZE`: used to setup and check the residual bitstream buffer
pub const SACDEC_RESIDUAL_BS_BUF_SIZE: i32 = 1024;

/// `MAX_NUM_PARAMS`
pub const MAX_NUM_PARAMS: usize = MAX_NUM_OTT + 4 * MAX_NUM_TTT + MAX_INPUT_CHANNELS;
/// `MAX_NUM_PARAMETERS`
pub const MAX_NUM_PARAMETERS: usize =
    if MAX_NUM_PARAMS > MAX_NUM_OTT { MAX_NUM_PARAMS } else { MAX_NUM_OTT };
/// `MAX_PARAMETER_SETS`
pub const MAX_PARAMETER_SETS: usize = 9;
/// `MAX_M2_INPUT`: 3 direct + 5 diffuse
pub const MAX_M2_INPUT: usize = MAX_OUTPUT_CHANNELS;
/// `MAX_QMF_BANDS_TO_HYBRID`: 3 bands are filtered again in the "40 bands" case
pub const MAX_QMF_BANDS_TO_HYBRID: i32 = 3;
/// `PROTO_LEN`
pub const PROTO_LEN: i32 = 13;
/// `BUFFER_LEN_LF`
pub const BUFFER_LEN_LF: i32 = PROTO_LEN;
/// `BUFFER_LEN_HF`
pub const BUFFER_LEN_HF: i32 = (PROTO_LEN - 1) / 2;
/// `MAX_NO_DECORR_CHANNELS`
pub const MAX_NO_DECORR_CHANNELS: usize = MAX_OUTPUT_CHANNELS;
/// `HRTF_AZIMUTHS`
pub const HRTF_AZIMUTHS: i32 = 5;
/// `MAX_NUM_OTT_AT`
pub const MAX_NUM_OTT_AT: usize = 0;

/// `UPMIXTYPE`
pub type Upmixtype = i32;
/// `UPMIXTYPE_BYPASS`: just bypass the input channels without processing
pub const UPMIXTYPE_BYPASS: Upmixtype = -1;
/// `UPMIXTYPE_NORMAL`: multichannel loudspeaker upmix with spatial data
pub const UPMIXTYPE_NORMAL: Upmixtype = 0;

/// `isTwoChMode(upmixType)`: always 0.
#[inline]
pub const fn is_two_ch_mode(_upmix_type: Upmixtype) -> bool {
    false
}

/// `MPEGS_BYPASSMODE`
pub const MPEGS_BYPASSMODE: u32 = 0x00000001;
/// `MPEGS_CONCEAL`
pub const MPEGS_CONCEAL: u32 = 0x00000002;

/// `SACDEC_SYNTAX_MPS`
pub const SACDEC_SYNTAX_MPS: u32 = 1;
/// `SACDEC_SYNTAX_USAC`
pub const SACDEC_SYNTAX_USAC: u32 = 2;
/// `SACDEC_SYNTAX_RSVD50`
pub const SACDEC_SYNTAX_RSVD50: u32 = 4;
/// `SACDEC_SYNTAX_L2`
pub const SACDEC_SYNTAX_L2: u32 = 8;
/// `SACDEC_SYNTAX_L3`
pub const SACDEC_SYNTAX_L3: u32 = 16;
/// `SACDEC_SYNTAX_LD`
pub const SACDEC_SYNTAX_LD: u32 = 32;

/// The sizes of the arrays of the only decoder configuration (`DECODER_LEVEL_0`); see
/// [`fdk_spatial_dec_open`].
pub const NUM_IN_CH: usize = 1;
/// `setup.maxNumOutputChannels`
pub const NUM_OUT_CH: usize = 2;
/// `setup.maxNumVChannels`
pub const NUM_V_CH: usize = 2;
/// `setup.maxNumOttBoxes`
pub const NUM_OTT: usize = 1;
/// `setup.maxNumQmfBands` (and `maxNumCmplxQmfBands`)
pub const NUM_QMF: usize = 64;
/// `setup.maxNumHybridBands` (and `maxNumCmplxHybBands`)
pub const NUM_HYB: usize = 71;
/// `setup.maxNumResChannels`
pub const NUM_RES_CH: usize = 1;
/// `setup.maxNumDecorChannels`
pub const NUM_DECOR: usize = 1;
/// `setup.maxNumInputChannels + setup.maxNumResChannels`: the hybrid analysis filter banks
pub const NUM_HYB_ANA: usize = NUM_IN_CH + NUM_RES_CH;
/// `2 * BUFFER_LEN_LF * MAX_QMF_BANDS_TO_HYBRID`: LF state memory of a hybrid analysis
pub const HYB_LF_SIZE: usize = (2 * BUFFER_LEN_LF * MAX_QMF_BANDS_TO_HYBRID) as usize;
/// `BUFFER_LEN_HF * ((maxNumQmfBands - 3) + (maxNumCmplxQmfBands - 3))`: HF state memory
pub const HYB_HF_SIZE: usize = (BUFFER_LEN_HF
    * ((NUM_QMF as i32 - MAX_QMF_BANDS_TO_HYBRID) + (NUM_QMF as i32 - MAX_QMF_BANDS_TO_HYBRID)))
    as usize;

/// `LOSSLESSSTATE`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LosslessState {
    /// `bsQuantCoarseXXXprev`
    pub bs_quant_coarse_xxx_prev: i8,
    /// `bsQuantCoarseXXXprevParse`
    pub bs_quant_coarse_xxx_prev_parse: i8,
}

/// `LOSSLESSDATA`. `state` (the link to the persistent state) is the entry with the same index
/// in the [`BsLlState`] (see the [module documentation](self)).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LosslessData {
    /// `bsXXXDataMode`
    pub bs_xxx_data_mode: [i8; MAX_PARAMETER_SETS],
    /// `bsQuantCoarseXXX`
    pub bs_quant_coarse_xxx: [i8; MAX_PARAMETER_SETS],
    /// `bsFreqResStrideXXX`
    pub bs_freq_res_stride_xxx: [i8; MAX_PARAMETER_SETS],
    /// `nocmpQuantCoarseXXX`
    pub nocmp_quant_coarse_xxx: [i8; MAX_PARAMETER_SETS],
}

/// One parameter matrix `[MAX_PARAMETER_SETS][MAX_PARAMETER_BANDS]` of `SCHAR`.
pub type ParamSets = [[i8; MAX_PARAMETER_BANDS]; MAX_PARAMETER_SETS];

/// `struct SPATIAL_BS_FRAME_struct` (`SPATIAL_BS_FRAME`)
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpatialBsFrame {
    /// `bsIndependencyFlag`
    pub bs_independency_flag: u8,
    /// `newBsData`
    pub new_bs_data: u8,
    /// `numParameterSets`
    pub num_parameter_sets: u8,
    /// `paramSlot`
    pub param_slot: [i32; MAX_PARAMETER_SETS],
    /// `cmpOttCLDidx`: compact indices, written by the parser in ecDataDec() and read during
    /// decode in mapIndexData()
    pub cmp_ott_cld_idx: [ParamSets; MAX_NUM_OTT + MAX_NUM_OTT_AT],
    /// `cmpOttICCidx`
    pub cmp_ott_icc_idx: [ParamSets; MAX_NUM_OTT],
    /// `bsSmoothMode`
    pub bs_smooth_mode: [u8; MAX_PARAMETER_SETS],
    /// `bsSmoothTime`
    pub bs_smooth_time: [u8; MAX_PARAMETER_SETS],
    /// `bsFreqResStrideSmg`
    pub bs_freq_res_stride_smg: [u8; MAX_PARAMETER_SETS],
    /// `bsSmgData`: smoothing flags, one if the band is smoothed, otherwise zero
    pub bs_smg_data: [[u8; MAX_PARAMETER_BANDS]; MAX_PARAMETER_SETS],
    /// `cmpArbdmxGainIdx` (allocated with `MAX_INPUT_CHANNELS` entries)
    pub cmp_arbdmx_gain_idx: [ParamSets; MAX_INPUT_CHANNELS],
    /// `CLDLosslessData` (allocated with `MAX_NUM_PARAMETERS` entries)
    pub cld_lossless_data: [LosslessData; MAX_NUM_PARAMETERS],
    /// `ICCLosslessData`
    pub icc_lossless_data: [LosslessData; MAX_NUM_PARAMETERS],
    /// `IPDLosslessData`
    pub ipd_lossless_data: [LosslessData; MAX_NUM_PARAMETERS],
    /// `cmpOttIPDidx` (allocated with `MAX_NUM_OTT` entries)
    pub cmp_ott_ipd_idx: [ParamSets; MAX_NUM_OTT],
    /// `phaseMode`
    pub phase_mode: i32,
    /// `OpdSmoothingMode`
    pub opd_smoothing_mode: i32,
    /// `tempShapeEnableChannelGES`: GES side info
    pub temp_shape_enable_channel_ges: [u8; MAX_OUTPUT_CHANNELS],
    /// `bsEnvShapeData`: GES side info (quantized)
    pub bs_env_shape_data: [[u8; MAX_TIME_SLOTS as usize]; MAX_OUTPUT_CHANNELS],
    /// `tempShapeEnableChannelSTP`: STP side info
    pub temp_shape_enable_channel_stp: [u8; MAX_OUTPUT_CHANNELS],
    /// `TsdData[1]`: TSD data structure
    pub tsd_data: [TsdData; 1],
}

impl Default for SpatialBsFrame {
    fn default() -> Self {
        let z: ParamSets = [[0; MAX_PARAMETER_BANDS]; MAX_PARAMETER_SETS];
        SpatialBsFrame {
            bs_independency_flag: 0,
            new_bs_data: 0,
            num_parameter_sets: 0,
            param_slot: [0; MAX_PARAMETER_SETS],
            cmp_ott_cld_idx: [z; MAX_NUM_OTT],
            cmp_ott_icc_idx: [z; MAX_NUM_OTT],
            bs_smooth_mode: [0; MAX_PARAMETER_SETS],
            bs_smooth_time: [0; MAX_PARAMETER_SETS],
            bs_freq_res_stride_smg: [0; MAX_PARAMETER_SETS],
            bs_smg_data: [[0; MAX_PARAMETER_BANDS]; MAX_PARAMETER_SETS],
            cmp_arbdmx_gain_idx: [z; MAX_INPUT_CHANNELS],
            cld_lossless_data: [LosslessData::default(); MAX_NUM_PARAMETERS],
            icc_lossless_data: [LosslessData::default(); MAX_NUM_PARAMETERS],
            ipd_lossless_data: [LosslessData::default(); MAX_NUM_PARAMETERS],
            cmp_ott_ipd_idx: [z; MAX_NUM_OTT],
            phase_mode: 0,
            opd_smoothing_mode: 0,
            temp_shape_enable_channel_ges: [0; MAX_OUTPUT_CHANNELS],
            bs_env_shape_data: [[0; MAX_TIME_SLOTS as usize]; MAX_OUTPUT_CHANNELS],
            temp_shape_enable_channel_stp: [0; MAX_OUTPUT_CHANNELS],
            tsd_data: [TsdData::default()],
        }
    }
}

/// `BS_LL_STATE`: the lossless states.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BsLlState {
    /// `CLDLosslessState`
    pub cld_lossless_state: [LosslessState; MAX_NUM_PARAMETERS],
    /// `ICCLosslessState`
    pub icc_lossless_state: [LosslessState; MAX_NUM_PARAMETERS],
    /// `IPDLosslessState`
    pub ipd_lossless_state: [LosslessState; MAX_NUM_PARAMETERS],
}

/// `SMOOTHING_STATE`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SmoothingState {
    /// `prevParamSlot`
    pub prev_param_slot: i32,
    /// `prevSmgTime`
    pub prev_smg_time: i32,
    /// `prevSmgData`
    pub prev_smg_data: [u8; MAX_PARAMETER_BANDS],
    /// `opdLeftState__FDK`
    pub opd_left_state__fdk: [FixpDbl; MAX_PARAMETER_BANDS],
    /// `opdRightState__FDK`
    pub opd_right_state__fdk: [FixpDbl; MAX_PARAMETER_BANDS],
}

/// `2 * MAX_OUTPUT_CHANNELS + MAX_INPUT_CHANNELS`: the channels of the GES state.
pub const BB_ENV_CHANNELS: usize = 2 * MAX_OUTPUT_CHANNELS + MAX_INPUT_CHANNELS;

/// `RESHAPE_BBENV_STATE`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReshapeBbEnvState {
    /// `alpha__FDK`
    pub alpha__fdk: FixpDbl,
    /// `beta__FDK`
    pub beta__fdk: FixpDbl,
    /// `partNrgPrev__FDK`
    pub part_nrg_prev__fdk: [[FixpDbl; BB_ENV_SIZE]; BB_ENV_CHANNELS],
    /// `normNrgPrev__FDK`
    pub norm_nrg_prev__fdk: [FixpDbl; BB_ENV_CHANNELS],
    /// `frameNrgPrev__FDK`
    pub frame_nrg_prev__fdk: [FixpDbl; BB_ENV_CHANNELS],
    /// `partNrgPrevSF`
    pub part_nrg_prev_sf: [i32; BB_ENV_CHANNELS],
    /// `partNrgPrev2SF`
    pub part_nrg_prev2_sf: [i32; BB_ENV_CHANNELS],
    /// `normNrgPrevSF`
    pub norm_nrg_prev_sf: [i32; BB_ENV_CHANNELS],
    /// `frameNrgPrevSF`
    pub frame_nrg_prev_sf: [i32; BB_ENV_CHANNELS],
}

/// `SACDEC_CREATION_PARAMS`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SacdecCreationParams {
    /// `maxNumInputChannels`
    pub max_num_input_channels: i32,
    /// `maxNumOutputChannels`
    pub max_num_output_channels: i32,
    /// `maxNumQmfBands`
    pub max_num_qmf_bands: i32,
    /// `maxNumHybridBands`
    pub max_num_hybrid_bands: i32,
    /// `maxNumXChannels`
    pub max_num_x_channels: i32,
    /// `maxNumVChannels`
    pub max_num_v_channels: i32,
    /// `maxNumDecorChannels`
    pub max_num_decor_channels: i32,
    /// `maxNumCmplxQmfBands`
    pub max_num_cmplx_qmf_bands: i32,
    /// `maxNumCmplxHybBands`
    pub max_num_cmplx_hyb_bands: i32,
    /// `maxNumResChannels`
    pub max_num_res_channels: i32,
    /// `bProcResidual`: process residual
    pub b_proc_residual: i32,
    /// `maxNumResidualChannels`
    pub max_num_residual_channels: i32,
    /// `maxNumOttBoxes`
    pub max_num_ott_boxes: i32,
    /// `maxNumParams`
    pub max_num_params: i32,
}

/// `struct spatialDec_struct` (`spatialDec`, `HANDLE_SPATIAL_DEC`). See the
/// [module documentation](self) for the memory and the pointers.
///
/// Members of the C structure that the library never uses (`hResidualBitstreams`,
/// `qmfInputRealDelayBuffer__FDK`, `qmfInputImagDelayBuffer__FDK`, `binInputReverb`,
/// `qmfOutputRealDryDelayBuffer__FDK`, `qmfOutputImagDryFilterBuffer__FDK`,
/// `qmfOutputImagDryFilterBufferBase__FDK`, `qmfOutputRealDryDelayBufPos`) are not ported.
#[derive(Clone, Debug)]
pub struct SpatialDec {
    /// `errInt`: internal errors, cleared at the beginning of each process call
    pub err_int: SacdecError,
    /// `staticDecScale`: static scale of the decoder
    pub static_dec_scale: i32,
    /// `samplingFreq` `Hz`
    pub sampling_freq: i32,
    /// `decoderLevel`
    pub decoder_level: CfgLevel,
    /// `decoderMode`
    pub decoder_mode: CfgExtent,
    /// `binauralMode`
    pub binaural_mode: CfgBinaural,
    /// `createParams`
    pub create_params: SacdecCreationParams,
    /// `numComplexProcessingBands`
    pub num_complex_processing_bands: i32,
    /// `treeConfig`
    pub tree_config: i32,
    /// `numInputChannels`
    pub num_input_channels: i32,
    /// `numOutputChannels`
    pub num_output_channels: i32,
    /// `numOttBoxes`
    pub num_ott_boxes: i32,
    /// `numM2rows`
    pub num_m2rows: i32,
    /// `numOutputChannelsAT`: number of output channels after arbitrary tree processing
    pub num_output_channels_at: i32,
    /// `quantMode`
    pub quant_mode: i32,
    /// `arbitraryDownmix`
    pub arbitrary_downmix: i32,
    /// `residualCoding`
    pub residual_coding: i32,
    /// `nrResidualFrame`
    pub nr_residual_frame: u8,
    /// `nrArbDownmixResidualFrame`
    pub nr_arb_downmix_residual_frame: u8,
    /// `tempShapeConfig`
    pub temp_shape_config: i32,
    /// `decorrType`: indicates to use the PS or the non-PS decorrelator
    pub decorr_type: i32,
    /// `decorrConfig`: chosen decorrelator
    pub decorr_config: i32,
    /// `envQuantMode`: quantization mode of the envelope reshaping data
    pub env_quant_mode: i32,
    /// `clipProtectGain__FDK`: global gain for upmix
    pub clip_protect_gain__fdk: FixpDbl,
    /// `clipProtectGainSF__FDK`: global gain for upmix (`char`)
    pub clip_protect_gain_sf__fdk: i8,
    /// `numDirektSignals`
    pub num_direkt_signals: i32,
    /// `wStartResidualIdx`
    pub w_start_residual_idx: i32,
    /// `numDecorSignals`
    pub num_decor_signals: i32,
    /// `numVChannels`: direct signals + decorrelator signals
    pub num_v_channels: i32,
    /// `numXChannels`: direct input signals + TTT residuals
    pub num_x_channels: i32,
    /// `timeSlots`: length of the spatial frame in QMF samples
    pub time_slots: i32,
    /// `curTimeSlot`: the current time slot used for hyperframing
    pub cur_time_slot: i32,
    /// `prevTimeSlot`
    pub prev_time_slot: i32,
    /// `curPs`
    pub cur_ps: i32,
    /// `frameLength`: number of output waveform samples/channel/frame
    pub frame_length: i32,
    /// `upmixType`
    pub upmix_type: Upmixtype,
    /// `partiallyComplex`
    pub partially_complex: i32,
    /// `useFDreverb`
    pub use_fd_reverb: i32,
    /// `bShareDelayWithSBR`
    pub b_share_delay_with_sbr: i32,
    /// `tp_hybBandBorder`: hybrid band indicating the HP filter cut-off
    pub tp_hyb_band_border: i32,
    /// `qmfBands`
    pub qmf_bands: i32,
    /// `hybridBands`
    pub hybrid_bands: i32,
    /// `kernels`: mapping of hybrid bands to parameter bands (`None` for `NULL`)
    pub kernels: Option<&'static [i8; MAX_HYBRID_BANDS]>,
    /// `TsdTs`: TSD QMF slot counter, `0 <= ts < numSlots`
    pub tsd_ts: i32,
    /// `param2hyb`: mapping of parameter bands to hybrid bands
    pub param2hyb: [i32; MAX_PARAMETER_BANDS + 1],
    /// `kernels_width`: mapping of parameter bands to hybrid band offsets
    pub kernels_width: [i32; MAX_PARAMETER_BANDS],
    /// `residualSamplingFreq`
    pub residual_sampling_freq: i32,
    /// `residualPresent`
    pub residual_present: [u8; MAX_NUM_OTT + MAX_NUM_TTT],
    /// `residualBands`: 0 if no residual data is present for this box
    pub residual_bands: [u8; MAX_NUM_OTT + MAX_NUM_TTT],
    /// `residualQMFBands`
    pub residual_qmf_bands: [u8; MAX_NUM_OTT + MAX_NUM_TTT],
    /// `pConfigCurrent != NULL` (see the [module documentation](self))
    pub p_config_current: bool,
    /// `arbdmxFramesPerSpatialFrame`
    pub arbdmx_frames_per_spatial_frame: i32,
    /// `arbdmxUpdQMF`
    pub arbdmx_upd_qmf: i32,
    /// `numParameterBands`
    pub num_parameter_bands: i32,
    /// `bitstreamParameterBands`
    pub bitstream_parameter_bands: i32,
    /// `numOttBands`: number of bands for each OTT
    pub num_ott_bands: [i32; NUM_OTT],
    /// `extendFrame`
    pub extend_frame: u8,
    /// `numParameterSetsPrev`
    pub num_parameter_sets_prev: u8,
    /// `smgTime`
    pub smg_time: [i32; MAX_PARAMETER_SETS],
    /// `smgData`
    pub smg_data: [[u8; MAX_PARAMETER_BANDS]; MAX_PARAMETER_SETS],
    /// `ottCLDidxPrev`
    pub ott_cld_idx_prev: [[i8; MAX_PARAMETER_BANDS]; NUM_OTT],
    /// `ottICCidxPrev`
    pub ott_icc_idx_prev: [[i8; MAX_PARAMETER_BANDS]; NUM_OTT],
    /// `arbdmxGainIdxPrev`
    pub arbdmx_gain_idx_prev: [[i8; MAX_PARAMETER_BANDS]; NUM_IN_CH],
    /// `ottIPDidxPrev`
    pub ott_ipd_idx_prev: [[i8; MAX_PARAMETER_BANDS]; NUM_OTT],
    /// `outIdxData`
    pub out_idx_data: [ParamSets; NUM_OTT],
    /// `cmpOttCLDidxPrev`
    pub cmp_ott_cld_idx_prev: [[i8; MAX_PARAMETER_BANDS]; NUM_OTT],
    /// `cmpOttICCidxPrev`
    pub cmp_ott_icc_idx_prev: [[i8; MAX_PARAMETER_BANDS]; NUM_OTT],
    /// `ottICCdiffidx`
    pub ott_icc_diffidx: [ParamSets; NUM_OTT],
    /// `cmpOttIPDidxPrev`
    pub cmp_ott_ipd_idx_prev: [[i8; MAX_PARAMETER_BANDS]; NUM_OTT],
    /// `cmpArbdmxGainIdxPrev`
    pub cmp_arbdmx_gain_idx_prev: [[i8; MAX_PARAMETER_BANDS]; NUM_IN_CH],
    /// `ottCLD__FDK`
    pub ott_cld__fdk: [ParamSets; NUM_OTT],
    /// `ottICC__FDK`
    pub ott_icc__fdk: [ParamSets; NUM_OTT],
    /// `arbdmxGain__FDK`: the artistic downmix correction index
    pub arbdmx_gain__fdk: [ParamSets; NUM_IN_CH],
    /// `arbdmxAlpha__FDK`
    pub arbdmx_alpha__fdk: [FixpDbl; NUM_IN_CH],
    /// `arbdmxAlphaPrev__FDK`
    pub arbdmx_alpha_prev__fdk: [FixpDbl; NUM_IN_CH],
    /// `stereoConfigIndex`
    pub stereo_config_index: u8,
    /// `highRateMode`
    pub high_rate_mode: i32,
    /// `phaseCoding`
    pub phase_coding: i32,
    /// `ottIPD__FDK`
    pub ott_ipd__fdk: [ParamSets; NUM_OTT],
    /// `PhaseLeft__FDK`
    pub phase_left__fdk: [FixpDbl; MAX_PARAMETER_BANDS],
    /// `PhaseRight__FDK`
    pub phase_right__fdk: [FixpDbl; MAX_PARAMETER_BANDS],
    /// `PhasePrevLeft__FDK`
    pub phase_prev_left__fdk: [FixpDbl; MAX_PARAMETER_BANDS],
    /// `PhasePrevRight__FDK`
    pub phase_prev_right__fdk: [FixpDbl; MAX_PARAMETER_BANDS],
    /// `numOttBandsIPD`
    pub num_ott_bands_ipd: i32,
    /// `M2Real__FDK`
    pub m2_real__fdk: [[[FixpDbl; MAX_PARAMETER_BANDS]; NUM_V_CH]; NUM_OUT_CH],
    /// `M2Imag__FDK`
    pub m2_imag__fdk: [[[FixpDbl; MAX_PARAMETER_BANDS]; NUM_V_CH]; NUM_OUT_CH],
    /// `M2RealPrev__FDK`
    pub m2_real_prev__fdk: [[[FixpDbl; MAX_PARAMETER_BANDS]; NUM_V_CH]; NUM_OUT_CH],
    /// `M2ImagPrev__FDK`
    pub m2_imag_prev__fdk: [[[FixpDbl; MAX_PARAMETER_BANDS]; NUM_V_CH]; NUM_OUT_CH],
    /// `pc_filterdelay`: additional delay to align HQ with LP before hybrid analysis
    pub pc_filterdelay: i32,
    /// `qmfInputDelayBufPos`
    pub qmf_input_delay_buf_pos: i32,
    /// `qmfInputReal__FDK`
    pub qmf_input_real__fdk: [[FixpDbl; NUM_QMF]; NUM_IN_CH],
    /// `qmfInputImag__FDK`
    pub qmf_input_imag__fdk: [[FixpDbl; NUM_QMF]; NUM_IN_CH],
    /// `hybInputReal__FDK`
    pub hyb_input_real__fdk: [[FixpDbl; NUM_HYB]; NUM_IN_CH],
    /// `hybInputImag__FDK`
    pub hyb_input_imag__fdk: [[FixpDbl; NUM_HYB]; NUM_IN_CH],
    /// `binGain`
    pub bin_gain: FixpDbl,
    /// `reverbGain`
    pub reverb_gain: FixpDbl,
    /// `binCenterGain`
    pub bin_center_gain: FixpDbl,
    /// `reverbCenterGain`
    pub reverb_center_gain: FixpDbl,
    /// `qmfResidualReal__FDK` (`[maxNumResChannels][1][maxNumQmfBands]`)
    pub qmf_residual_real__fdk: [[[FixpDbl; NUM_QMF]; 1]; NUM_RES_CH],
    /// `qmfResidualImag__FDK`
    pub qmf_residual_imag__fdk: [[[FixpDbl; NUM_QMF]; 1]; NUM_RES_CH],
    /// `hybResidualReal__FDK`
    pub hyb_residual_real__fdk: [[FixpDbl; NUM_HYB]; NUM_RES_CH],
    /// `hybResidualImag__FDK`
    pub hyb_residual_imag__fdk: [[FixpDbl; NUM_HYB]; NUM_RES_CH],
    /// `wReal__FDK`
    pub w_real__fdk: [[FixpDbl; NUM_HYB]; NUM_V_CH],
    /// `wImag__FDK`
    pub w_imag__fdk: [[FixpDbl; NUM_HYB]; NUM_V_CH],
    /// `hybOutputRealDry__FDK`
    pub hyb_output_real_dry__fdk: [[FixpDbl; NUM_HYB]; NUM_OUT_CH],
    /// `hybOutputImagDry__FDK`
    pub hyb_output_imag_dry__fdk: [[FixpDbl; NUM_HYB]; NUM_OUT_CH],
    /// `hybOutputRealWet__FDK`
    pub hyb_output_real_wet__fdk: [[FixpDbl; NUM_HYB]; NUM_OUT_CH],
    /// `hybOutputImagWet__FDK`
    pub hyb_output_imag_wet__fdk: [[FixpDbl; NUM_HYB]; NUM_OUT_CH],
    /// `hybridAnalysis`: the analysis hybrid filter banks (input channels, then residual
    /// channels); they own their state memory (`pHybridAnaStates{LF,HF}{dmx,res}`)
    pub hybrid_analysis: [FdkAnaHybFilter; NUM_HYB_ANA],
    /// `hybridSynthesis`: the synthesis hybrid filter banks
    pub hybrid_synthesis: [FdkSynHybFilter; NUM_OUT_CH],
    /// `apDecor`: the decorrelators; they own their buffers (`pDecorBufferCplx`)
    pub ap_decor: [DecorrDec; NUM_DECOR],
    /// `smoothState`
    pub smooth_state: SmoothingState,
    /// `reshapeBBEnvState`: GES state
    pub reshape_bb_env_state: ReshapeBbEnvState,
    /// `row2channelDmxGES`
    pub row2channel_dmx_ges: [i8; MAX_OUTPUT_CHANNELS],
    /// `hStpDec`: STP state
    pub h_stp_dec: StpDec,
    /// `pActivM2ParamBands != NULL` (never set by the library)
    pub p_activ_m2_param_bands: bool,
    /// `bOverwriteM1M2prev`: overwrite the previous M1/M2 parameters with the first set of the
    /// new frame after an SSC change
    pub b_overwrite_m1m2prev: i32,
    /// `concealInfo`
    pub conceal_info: SpatialDecConcealmentInfo,
    /// `sacInDataHeadroom`: headroom of the SAC input time signal to prevent clipping
    pub sac_in_data_headroom: i32,
}

impl Default for SpatialDec {
    /// The zero-initialized structure (`FDK_ALLOCATE_MEMORY_1D_INT` uses `FDKcalloc`).
    fn default() -> Self {
        let z: ParamSets = [[0; MAX_PARAMETER_BANDS]; MAX_PARAMETER_SETS];
        let zb = [0 as FixpDbl; MAX_PARAMETER_BANDS];
        let zh = [0 as FixpDbl; NUM_HYB];
        SpatialDec {
            err_int: 0,
            static_dec_scale: 0,
            sampling_freq: 0,
            decoder_level: 0,
            decoder_mode: 0,
            binaural_mode: 0,
            create_params: SacdecCreationParams::default(),
            num_complex_processing_bands: 0,
            tree_config: 0,
            num_input_channels: 0,
            num_output_channels: 0,
            num_ott_boxes: 0,
            num_m2rows: 0,
            num_output_channels_at: 0,
            quant_mode: 0,
            arbitrary_downmix: 0,
            residual_coding: 0,
            nr_residual_frame: 0,
            nr_arb_downmix_residual_frame: 0,
            temp_shape_config: 0,
            decorr_type: 0,
            decorr_config: 0,
            env_quant_mode: 0,
            clip_protect_gain__fdk: 0,
            clip_protect_gain_sf__fdk: 0,
            num_direkt_signals: 0,
            w_start_residual_idx: 0,
            num_decor_signals: 0,
            num_v_channels: 0,
            num_x_channels: 0,
            time_slots: 0,
            cur_time_slot: 0,
            prev_time_slot: 0,
            cur_ps: 0,
            frame_length: 0,
            upmix_type: 0,
            partially_complex: 0,
            use_fd_reverb: 0,
            b_share_delay_with_sbr: 0,
            tp_hyb_band_border: 0,
            qmf_bands: 0,
            hybrid_bands: 0,
            kernels: None,
            tsd_ts: 0,
            param2hyb: [0; MAX_PARAMETER_BANDS + 1],
            kernels_width: [0; MAX_PARAMETER_BANDS],
            residual_sampling_freq: 0,
            residual_present: [0; MAX_NUM_OTT + MAX_NUM_TTT],
            residual_bands: [0; MAX_NUM_OTT + MAX_NUM_TTT],
            residual_qmf_bands: [0; MAX_NUM_OTT + MAX_NUM_TTT],
            p_config_current: false,
            arbdmx_frames_per_spatial_frame: 0,
            arbdmx_upd_qmf: 0,
            num_parameter_bands: 0,
            bitstream_parameter_bands: 0,
            num_ott_bands: [0; NUM_OTT],
            extend_frame: 0,
            num_parameter_sets_prev: 0,
            smg_time: [0; MAX_PARAMETER_SETS],
            smg_data: [[0; MAX_PARAMETER_BANDS]; MAX_PARAMETER_SETS],
            ott_cld_idx_prev: [[0; MAX_PARAMETER_BANDS]; NUM_OTT],
            ott_icc_idx_prev: [[0; MAX_PARAMETER_BANDS]; NUM_OTT],
            arbdmx_gain_idx_prev: [[0; MAX_PARAMETER_BANDS]; NUM_IN_CH],
            ott_ipd_idx_prev: [[0; MAX_PARAMETER_BANDS]; NUM_OTT],
            out_idx_data: [z; NUM_OTT],
            cmp_ott_cld_idx_prev: [[0; MAX_PARAMETER_BANDS]; NUM_OTT],
            cmp_ott_icc_idx_prev: [[0; MAX_PARAMETER_BANDS]; NUM_OTT],
            ott_icc_diffidx: [z; NUM_OTT],
            cmp_ott_ipd_idx_prev: [[0; MAX_PARAMETER_BANDS]; NUM_OTT],
            cmp_arbdmx_gain_idx_prev: [[0; MAX_PARAMETER_BANDS]; NUM_IN_CH],
            ott_cld__fdk: [z; NUM_OTT],
            ott_icc__fdk: [z; NUM_OTT],
            arbdmx_gain__fdk: [z; NUM_IN_CH],
            arbdmx_alpha__fdk: [0; NUM_IN_CH],
            arbdmx_alpha_prev__fdk: [0; NUM_IN_CH],
            stereo_config_index: 0,
            high_rate_mode: 0,
            phase_coding: 0,
            ott_ipd__fdk: [z; NUM_OTT],
            phase_left__fdk: zb,
            phase_right__fdk: zb,
            phase_prev_left__fdk: zb,
            phase_prev_right__fdk: zb,
            num_ott_bands_ipd: 0,
            m2_real__fdk: [[zb; NUM_V_CH]; NUM_OUT_CH],
            m2_imag__fdk: [[zb; NUM_V_CH]; NUM_OUT_CH],
            m2_real_prev__fdk: [[zb; NUM_V_CH]; NUM_OUT_CH],
            m2_imag_prev__fdk: [[zb; NUM_V_CH]; NUM_OUT_CH],
            pc_filterdelay: 0,
            qmf_input_delay_buf_pos: 0,
            qmf_input_real__fdk: [[0; NUM_QMF]; NUM_IN_CH],
            qmf_input_imag__fdk: [[0; NUM_QMF]; NUM_IN_CH],
            hyb_input_real__fdk: [zh; NUM_IN_CH],
            hyb_input_imag__fdk: [zh; NUM_IN_CH],
            bin_gain: 0,
            reverb_gain: 0,
            bin_center_gain: 0,
            reverb_center_gain: 0,
            qmf_residual_real__fdk: [[[0; NUM_QMF]; 1]; NUM_RES_CH],
            qmf_residual_imag__fdk: [[[0; NUM_QMF]; 1]; NUM_RES_CH],
            hyb_residual_real__fdk: [zh; NUM_RES_CH],
            hyb_residual_imag__fdk: [zh; NUM_RES_CH],
            w_real__fdk: [zh; NUM_V_CH],
            w_imag__fdk: [zh; NUM_V_CH],
            hyb_output_real_dry__fdk: [zh; NUM_OUT_CH],
            hyb_output_imag_dry__fdk: [zh; NUM_OUT_CH],
            hyb_output_real_wet__fdk: [zh; NUM_OUT_CH],
            hyb_output_imag_wet__fdk: [zh; NUM_OUT_CH],
            hybrid_analysis: Default::default(),
            hybrid_synthesis: Default::default(),
            ap_decor: Default::default(),
            smooth_state: SmoothingState::default(),
            reshape_bb_env_state: ReshapeBbEnvState::default(),
            row2channel_dmx_ges: [0; MAX_OUTPUT_CHANNELS],
            h_stp_dec: StpDec::default(),
            p_activ_m2_param_bands: false,
            b_overwrite_m1m2prev: 0,
            conceal_info: SpatialDecConcealmentInfo::default(),
            sac_in_data_headroom: 0,
        }
    }
}

/// `GetProcBand(self, qs)`
#[inline]
pub fn get_proc_band(self_: &SpatialDec, qs: i32) -> i32 {
    self_.kernels.and_then(|k| k.get(qs as usize)).map_or(0, |&v| v as i32)
}

/// `kernels[i]` of the decoder (0 without a table or outside of it).
#[inline]
pub(crate) fn kernel_at(self_: &SpatialDec, i: i32) -> i32 {
    get_proc_band(self_, i)
}

/// `APPLY_M2_NONE`: init value
const APPLY_M2_NONE: i32 = 0;
/// `APPLY_M2`: apply m2 fallback implementation
const APPLY_M2: i32 = 1;
/// `APPLY_M2_MODE212`: apply m2 for 212 mode
const APPLY_M2_MODE212: i32 = 2;
/// `APPLY_M2_MODE212_Res_PhaseCoding`: apply m2 for 212 mode with residuals and phase coding
const APPLY_M2_MODE212_RES_PHASE_CODING: i32 = 3;

/// `FDK_SpatialDecInitDefaultSpatialSpecificConfig(pSpatialSpecificConfig, coreCodec,
/// coreChannels, samplingFreq, nTimeSlots, decoderLevel, isBlind)`
pub fn fdk_spatial_dec_init_default_spatial_specific_config(
    p_ssc: &mut SpatialSpecificConfig,
    core_codec: i32,
    core_channels: i32,
    sampling_freq: i32,
    n_time_slots: i32,
    decoder_level: i32,
    is_blind: i32,
) -> i32 {
    spatial_dec_default_specific_config(
        p_ssc,
        core_codec,
        sampling_freq,
        n_time_slots,
        decoder_level,
        is_blind,
        core_channels,
    )
}

/// `FDK_SpatialDecCompareSpatialSpecificConfigHeader(pSsc1, pSsc2)`: 0 if equal, else
/// `MPS_UNEQUAL_SSC` (every byte must be equal, `FDKmemcmp`).
pub fn fdk_spatial_dec_compare_spatial_specific_config_header(
    p_ssc1: &SpatialSpecificConfig,
    p_ssc2: &SpatialSpecificConfig,
) -> i32 {
    let mut result = MPS_OK;

    // we assume: every bit must be equal
    if p_ssc1.as_bytes() != p_ssc2.as_bytes() {
        result = MPS_UNEQUAL_SSC;
    }
    result
}

/// `SpatialDecClearFrameData(self, bsFrame, setup)`: clears/fakes the frame data to avoid
/// misconfiguration and allow proper error concealment.
fn spatial_dec_clear_frame_data(
    self_: &mut SpatialDec,
    bs_frame: &mut SpatialBsFrame,
    setup: &SacdecCreationParams,
) {
    // do not apply shaping tools (GES or STP)
    for i in 0..(setup.max_num_output_channels.max(0) as usize).min(MAX_OUTPUT_CHANNELS) {
        // MAX_OUTPUT_CHANNELS
        bs_frame.temp_shape_enable_channel_stp[i] = 0;
        bs_frame.temp_shape_enable_channel_ges[i] = 0;
    }

    bs_frame.tsd_data[0].bs_tsd_enable = 0;

    // use only 1 parameter set at the end of the frame
    bs_frame.num_parameter_sets = 1;
    bs_frame.param_slot[0] = self_.time_slots - 1;

    // parameter smoothing tool set to off
    bs_frame.bs_smooth_mode[0] = 0;
    init_parameter_smoothing(self_);

    // reset residual data
    {
        let res_time_slots = 1;
        let res_qmf_bands = (setup.max_num_qmf_bands.max(0) as usize).min(NUM_QMF);
        // `i < setup->bProcResidual ? fMin(...) : 0` parses as
        // `(i < bProcResidual) ? fMin(...) : 0`: the loop runs for i == 0 only
        // (bProcResidual == 1).
        let mut i = 0;
        while (if i < setup.b_proc_residual {
            f_min(
                setup.max_num_res_channels,
                setup.max_num_ott_boxes + setup.max_num_input_channels,
            )
        } else {
            0
        }) != 0
        {
            if (i as usize) < NUM_RES_CH {
                for j in 0..res_time_slots {
                    for k in 0..res_qmf_bands {
                        self_.qmf_residual_real__fdk[i as usize][j][k] = 0;
                        self_.qmf_residual_imag__fdk[i as usize][j][k] = 0;
                    }
                }
            }
            i += 1;
        }
    }
}

/// `FDK_SpatialDecOpen(config, stereoConfigIndex)`: `None` for an unsupported decoder level.
pub fn fdk_spatial_dec_open(
    config: &SpatialDecConfig,
    _stereo_config_index: i32,
) -> Option<Box<SpatialDec>> {
    let mut setup = SacdecCreationParams::default();

    match config.decoder_level {
        DECODER_LEVEL_0 => {
            // 212 maxNumOutputChannels== 2
            setup.max_num_input_channels = 1;
            setup.max_num_output_channels = 2;
            setup.max_num_qmf_bands = 64;
            setup.max_num_x_channels = 2;
            setup.max_num_v_channels = 2;
            setup.max_num_decor_channels = 1;
            setup.b_proc_residual = 1;
            setup.max_num_residual_channels = 0;
            setup.max_num_ott_boxes = 1;
            setup.max_num_params = setup.max_num_input_channels + setup.max_num_ott_boxes;
        }
        _ => return None,
    }

    setup.max_num_res_channels = 1;

    if config.max_num_output_channels == OUTPUT_CHANNELS_2_0 {
        setup.max_num_output_channels = f_min(setup.max_num_output_channels, 2);
    }

    setup.max_num_hybrid_bands = sac_get_hybrid_subbands(setup.max_num_qmf_bands);

    match config.decoder_mode {
        EXT_HQ_ONLY => {
            setup.max_num_cmplx_qmf_bands = setup.max_num_qmf_bands;
            setup.max_num_cmplx_hyb_bands = setup.max_num_hybrid_bands;
        }
        _ => {
            setup.max_num_cmplx_qmf_bands = fix_max(PC_NUM_BANDS, setup.max_num_qmf_bands);
            setup.max_num_cmplx_hyb_bands = fix_max(PC_NUM_HYB_BANDS, setup.max_num_hybrid_bands);
        }
    } // switch config->decoderMode

    let mut self_ = Box::<SpatialDec>::default();

    self_.create_params = setup;

    // The arrays are members of the structure (see the module documentation).

    let lf_size = (2 * BUFFER_LEN_LF * MAX_QMF_BANDS_TO_HYBRID) as usize;
    let hf_size = (BUFFER_LEN_HF
        * ((setup.max_num_qmf_bands - MAX_QMF_BANDS_TO_HYBRID)
            + (setup.max_num_cmplx_qmf_bands - MAX_QMF_BANDS_TO_HYBRID)))
        as usize;
    for i in 0..NUM_IN_CH {
        fdk_hybrid_analysis_open(
            &mut self_.hybrid_analysis[i],
            vec![0; lf_size],
            (lf_size * core::mem::size_of::<FixpDbl>()) as u32,
            vec![0; hf_size],
            (hf_size * core::mem::size_of::<FixpDbl>()) as u32,
        );
    }
    if setup.b_proc_residual != 0 {
        let lf_size = (2 * BUFFER_LEN_LF * MAX_QMF_BANDS_TO_HYBRID) as usize;
        let res_qmf =
            if config.decoder_mode == EXT_LP_ONLY { PC_NUM_BANDS } else { setup.max_num_qmf_bands };
        let hf_size = (BUFFER_LEN_HF
            * ((res_qmf - MAX_QMF_BANDS_TO_HYBRID)
                + (setup.max_num_cmplx_qmf_bands - MAX_QMF_BANDS_TO_HYBRID)))
            as usize;
        for i in NUM_IN_CH..NUM_IN_CH + NUM_RES_CH {
            fdk_hybrid_analysis_open(
                &mut self_.hybrid_analysis[i],
                vec![0; lf_size],
                (lf_size * core::mem::size_of::<FixpDbl>()) as u32,
                vec![0; hf_size],
                (hf_size * core::mem::size_of::<FixpDbl>()) as u32,
            );
        }
    }

    for i in 0..NUM_DECOR {
        if fdk_decorrelate_open(
            &mut self_.ap_decor[i],
            vec![0; DECORR_BUFFER_LEN as usize].into_boxed_slice(),
        ) != 0
        {
            return None;
        }
    }

    if subband_tp_create(&mut self_.h_stp_dec) != MPS_OK {
        return None;
    }

    // save general decoder configuration
    self_.decoder_level = config.decoder_level;
    self_.decoder_mode = config.decoder_mode;
    self_.binaural_mode = config.binaural_mode;

    // preinitialize configuration
    self_.partially_complex = (config.decoder_mode != EXT_HQ_ONLY) as i32;

    // Set to default state
    spatial_dec_concealment_init(&mut self_.conceal_info, MPEGS_CONCEAL_RESET_ALL);

    // Everything is fine so return the handle
    Some(self_)
}

/// `isValidConfig(self, upmixType, pUserParams, coreAot)`: 1 if the configuration is
/// supported, else 0.
fn is_valid_config(
    _self: &SpatialDec,
    upmix_type: SpatialDecUpmixType,
    _p_user_params: &SpatialdecParam,
    _core_aot: i32,
) -> i32 {
    match upmix_type {
        UPMIXTYPE_BYPASS | UPMIXTYPE_NORMAL => 1, // upmixType supported
        _ => 0,                                   // unsupported upmixType
    }
}

/// `CheckLevelTreeUpmixType(pCreateParams, pSsc, decoderLevel, upmixType)`
fn check_level_tree_upmix_type(
    p_create_params: &SacdecCreationParams,
    p_ssc: &SpatialSpecificConfig,
    decoder_level: i32,
    upmix_type: Upmixtype,
) -> SacdecError {
    let mut err = MPS_OK;

    let tree_config = p_ssc.tree_config;

    match decoder_level {
        0 => {
            if tree_config != SPATIALDEC_MODE_RSVD7 {
                return MPS_INVALID_TREECONFIG;
            }
        }
        _ => return MPS_INVALID_PARAMETER, // MPS_UNIMPLEMENTED
    }

    let n_output_channels = match upmix_type {
        UPMIXTYPE_BYPASS => p_ssc.n_input_channels,
        _ => p_ssc.n_output_channels,
    };

    // Is sufficient memory allocated.
    if p_ssc.n_input_channels > p_create_params.max_num_input_channels
        || n_output_channels > p_create_params.max_num_output_channels
        || p_ssc.n_ott_boxes > p_create_params.max_num_ott_boxes
    {
        err = MPS_INVALID_PARAMETER;
    }

    err
}

/// `SpatialDecInitParserContext(self)`: initializes the state variables of the MPS parser.
pub fn spatial_dec_init_parser_context(self_: &mut SpatialDec) {
    for i in 0..(self_.create_params.max_num_ott_boxes.max(0) as usize).min(NUM_OTT) {
        for j in 0..MAX_PARAMETER_BANDS {
            self_.ott_cld_idx_prev[i][j] = 0;
            self_.ott_icc_idx_prev[i][j] = 0;
            self_.cmp_ott_cld_idx_prev[i][j] = 0;
            self_.cmp_ott_icc_idx_prev[i][j] = 0;
        }
    }
    for i in 0..(self_.create_params.max_num_input_channels.max(0) as usize).min(NUM_IN_CH) {
        for j in 0..MAX_PARAMETER_BANDS {
            self_.arbdmx_gain_idx_prev[i][j] = 0;
            self_.cmp_arbdmx_gain_idx_prev[i][j] = 0;
        }
    }
}

/// `FDK_SpatialDecInit(self, frame, pSpatialSpecificConfig, nQmfBands, upmixType,
/// pUserParams, initFlags)`: initializes the decoder (may happen after the first parse
/// operation). `p_ssc` is also `self->pConfigCurrent` (see the module documentation); `qd` is
/// `self->pQmfDomain`.
pub fn fdk_spatial_dec_init(
    self_: &mut SpatialDec,
    qd: &mut FdkQmfDomain,
    _frame: &mut SpatialBsFrame,
    p_ssc: &mut SpatialSpecificConfig,
    n_qmf_bands: i32,
    upmix_type: SpatialDecUpmixType,
    p_user_params: &SpatialdecParam,
    init_flags: u32,
) -> SacdecError {
    let mut err;
    let mut bypass_mode = 0;

    self_.use_fd_reverb = 0;

    // check configuration parameter
    if is_valid_config(self_, upmix_type, p_user_params, p_ssc.core_codec) == 0 {
        return MPS_INVALID_PARAMETER;
    }

    // check tree configuration
    err = check_level_tree_upmix_type(&self_.create_params, p_ssc, self_.decoder_level, upmix_type);
    if err != MPS_OK {
        return err;
    }

    // Store and update instance after all checks passed successfully:
    self_.upmix_type = upmix_type;

    if init_flags & MPEGS_INIT_PARAMS_ERROR_CONCEALMENT != 0 {
        // At least one error concealment parameter changed
        let params = [
            (SAC_DEC_CONCEAL_METHOD, p_user_params.conceal_method as i32),
            (SAC_DEC_CONCEAL_NUM_KEEP_FRAMES, p_user_params.conceal_num_keep_frames as i32),
            (
                SAC_DEC_CONCEAL_FADE_OUT_SLOPE_LENGTH,
                p_user_params.conceal_fade_out_slope_length as i32,
            ),
            (
                SAC_DEC_CONCEAL_FADE_IN_SLOPE_LENGTH,
                p_user_params.conceal_fade_in_slope_length as i32,
            ),
            (SAC_DEC_CONCEAL_NUM_RELEASE_FRAMES, p_user_params.conceal_num_release_frames as i32),
        ];
        for (param, value) in params {
            err = spatial_dec_concealment_set_param(Some(&mut self_.conceal_info), param, value);
            if err != MPS_OK {
                return err;
            }
        }
    }

    if init_flags & MPEGS_INIT_STATES_ERROR_CONCEALMENT != 0 {
        // Set to default state
        spatial_dec_concealment_init(&mut self_.conceal_info, MPEGS_CONCEAL_RESET_STATE);
    }

    // determine bypass mode
    bypass_mode |= p_user_params.bypass_mode as i32;
    bypass_mode |= (self_.upmix_type == UPMIXTYPE_BYPASS) as i32;

    // static decoder scale depends on number of qmf bands
    match n_qmf_bands {
        16 | 24 | 32 => self_.static_dec_scale = 21,
        64 => self_.static_dec_scale = 22,
        _ => return MPS_INVALID_PARAMETER,
    }

    self_.num_parameter_sets_prev = 1;

    self_.qmf_bands = n_qmf_bands;
    // self->hybridBands will be updated in SpatialDecDecodeHeader() below.

    self_.b_share_delay_with_sbr = 0;

    err = spatial_dec_decode_header(self_, p_ssc);
    if err != MPS_OK {
        return err;
    }

    self_.stereo_config_index = p_ssc.stereo_config_index;

    if init_flags & MPEGS_INIT_STATES_ANA_QMF_FILTER != 0 {
        self_.qmf_input_delay_buf_pos = 0;
        self_.pc_filterdelay = 1; // Division by 0 not possible
    }

    let max_qmf_bands = self_.qmf_bands;

    // init residual decoder

    // init tonality smoothing
    if init_flags & MPEGS_INIT_STATES_PARAM != 0 {
        init_parameter_smoothing(self_);
    }

    // init GES
    init_bb_env(self_, (init_flags & MPEGS_INIT_STATES_GES != 0) as i32);

    // Clip protection is applied only for normal processing.
    if !is_two_ch_mode(self_.upmix_type) && bypass_mode == 0 {
        self_.static_dec_scale += self_.clip_protect_gain_sf__fdk as i32;
    }

    {
        let init_states_flag = (init_flags & MPEGS_INIT_STATES_ANA_QMF_FILTER != 0) as i32;
        let use_ld_filter = (p_ssc.syntax_flags & SACDEC_SYNTAX_LD != 0) as i32;

        let mut flags = qd.global_conf.flags_requested;
        flags &= !QMF_FLAG_LP;

        if init_states_flag != 0 {
            flags &= !QMF_FLAG_KEEP_STATES;
        } else {
            flags |= QMF_FLAG_KEEP_STATES;
        }

        if use_ld_filter != 0 {
            flags |= QMF_FLAG_MPSLDFB;
        } else {
            flags &= !QMF_FLAG_MPSLDFB;
        }

        qd.global_conf.flags_requested = flags;
        fdk_qmf_domain_configure(qd);

        // output scaling
        for n_ch in 0..(self_.num_output_channels_at.max(0) as usize).min(QMF_DOMAIN_MAX_CHANNELS) {
            let mut output_scale = 0;
            let mut scale = -(8) + (1);

            let (output_gain_m, output_gain_e) = get_ch_gain(self_, n_ch as u32);

            if !is_two_ch_mode(self_.upmix_type) && bypass_mode == 0 {
                // consider clip protection scaling at synthesis qmf
                output_scale += self_.clip_protect_gain_sf__fdk as i32;
            }

            scale += output_scale;

            let qd_out = &mut qd.qmf_domain_out[n_ch];
            let states = qd_out.p_syn_qmf_states.as_deref_mut().unwrap_or(&mut []);
            qmf_change_out_scalefactor(&mut qd_out.fb, states, scale);
            qmf_change_out_gain(&mut qd_out.fb, output_gain_m, output_gain_e);
        }
    }

    for n_ch in 0..(self_.num_output_channels_at.max(0) as usize).min(NUM_OUT_CH) {
        fdk_hybrid_synthesis_init(
            &mut self_.hybrid_synthesis[n_ch],
            THREE_TO_TEN,
            self_.qmf_bands,
            max_qmf_bands,
        );
    }

    // for input, residual channels and arbitrary down-mix residual channels
    let mut n_ch = 0usize;
    while n_ch < (self_.create_params.max_num_input_channels.max(0) as usize).min(NUM_HYB_ANA) {
        fdk_hybrid_analysis_init(
            &mut self_.hybrid_analysis[n_ch],
            THREE_TO_TEN,
            self_.qmf_bands,
            max_qmf_bands,
            (init_flags & MPEGS_INIT_STATES_ANA_HYB_FILTER != 0) as i32,
        );
        n_ch += 1;
    }
    let n_ana = if self_.create_params.b_proc_residual != 0 {
        self_.create_params.max_num_input_channels + self_.create_params.max_num_res_channels
    } else {
        self_.create_params.max_num_input_channels
    };
    while n_ch < (n_ana.max(0) as usize).min(NUM_HYB_ANA) {
        fdk_hybrid_analysis_init(
            &mut self_.hybrid_analysis[n_ch],
            THREE_TO_TEN,
            max_qmf_bands,
            max_qmf_bands,
            0,
        );
        n_ch += 1;
    }

    {
        for k in 0..(self_.num_decor_signals.max(0) as usize).min(NUM_DECOR) {
            let mut decorr_type = DECORR_LD;
            if p_ssc.syntax_flags & (SACDEC_SYNTAX_USAC | SACDEC_SYNTAX_RSVD50) != 0 {
                decorr_type = if self_.tree_config == TREE_212 && self_.decorr_type == DECORR_PS {
                    DECORR_PS
                } else {
                    DECORR_USAC
                };
            }
            let mut idec = k as i32;
            if p_ssc.syntax_flags & SACDEC_SYNTAX_LD != 0 && self_.tree_config == TREE_212 && k == 0
            {
                idec = 2;
            }
            let err_code = fdk_decorrelate_init(
                &mut self_.ap_decor[k],
                self_.hybrid_bands,
                decorr_type,
                DUCKER_AUTOMATIC,
                self_.decorr_config,
                idec,
                0, // self->partiallyComplex
                0,
                0, // isLegacyPS
                (init_flags & MPEGS_INIT_STATES_DECORRELATOR != 0) as i32,
            );
            if err_code != 0 {
                return MPS_NOTOK;
            }
        }
    } // !self->partiallyComplex

    err = init_m1_and_m2(
        self_,
        (init_flags & MPEGS_INIT_STATES_M1M2 != 0) as i32,
        (init_flags & MPEGS_INIT_CONFIG != 0) as i32,
    );
    if err != MPS_OK {
        return err;
    }

    // Initialization of previous frame data
    if init_flags & MPEGS_INIT_STATES_PARAM != 0 {
        for i in 0..(self_.create_params.max_num_ott_boxes.max(0) as usize).min(NUM_OTT) {
            // reset icc diff data
            for k in 0..MAX_PARAMETER_SETS {
                for j in 0..MAX_PARAMETER_BANDS {
                    self_.ott_icc_diffidx[i][k][j] = 0;
                }
            }
        }
        // Parameter Smoothing
        // robustness: init with one of the values of smgTimeTable[] = {64, 128, 256, 512} to
        // avoid division by zero in calcFilterCoeff__FDK()
        self_.smooth_state.prev_smg_time = SMG_TIME_TABLE[2]; // == 256
        self_.smooth_state.prev_smg_data = [0; MAX_PARAMETER_BANDS];
        self_.smooth_state.opd_left_state__fdk = [0; MAX_PARAMETER_BANDS];
        self_.smooth_state.opd_right_state__fdk = [0; MAX_PARAMETER_BANDS];
    }

    self_.prev_time_slot = -1;
    // Initialize with a invalid value to trigger concealment if first frame has no valid data.
    self_.cur_time_slot = MAX_TIME_SLOTS + 1;
    self_.cur_ps = 0;

    subband_tp_init(&mut self_.h_stp_dec);

    err
}

/// `SpatialDecChannelProperties(self, channelType, channelIndices, mapDescr)`: fills the
/// arrays with the channel types and indices of the output channels (`None` for `NULL`).
pub fn spatial_dec_channel_properties(
    self_: Option<&SpatialDec>,
    channel_type: Option<&mut [fdk_aac_rs_fdk::audio::AudioChannelType]>,
    channel_indices: Option<&mut [u8]>,
    map_descr: Option<&FdkChannelMapDescr>,
) {
    use fdk_aac_rs_fdk::audio::ACT_FRONT;
    let (Some(self_), Some(channel_type), Some(channel_indices), Some(_)) =
        (self_, channel_type, channel_indices, map_descr)
    else {
        return; // no extern buffer to be filled
    };

    if self_.num_output_channels_at != tree_property(self_.tree_config).num_output_channels as i32 {
        // Declare all channels to be front channels:
        for ch in 0..self_.num_output_channels_at.max(0) as usize {
            // Deviation: entries beyond the caller's arrays are not written.
            if let Some(t) = channel_type.get_mut(ch) {
                *t = ACT_FRONT;
            }
            if let Some(i) = channel_indices.get_mut(ch) {
                *i = ch as u8;
            }
        }
    } else {
        // ISO/IEC FDIS 23003-1:2006(E), page 46, Table 40 bsTreeConfig
        if self_.tree_config == TREE_212 && channel_type.len() >= 2 && channel_indices.len() >= 2 {
            channel_type[0] = ACT_FRONT;
            channel_indices[0] = 0;
            channel_type[1] = ACT_FRONT;
            channel_indices[1] = 1;
        }
    }
}

/// `FDK_SpatialDecClose(self)`: releases the decoder (the members free themselves).
pub fn fdk_spatial_dec_close(self_: Option<Box<SpatialDec>>) {
    if let Some(mut s) = self_ {
        for k in 0..NUM_DECOR {
            fdk_decorrelate_close(&mut s.ap_decor[k]);
        }
        subband_tp_destroy(&mut s.h_stp_dec);
    }
}

/// `SpatialDecApplyBypass(self, hybInputReal, hybInputImag, hybOutputReal, hybOutputImag,
/// numInputChannels)`: the surround bypass copies (`self->hybInput*` to
/// `self->hybOutput*Dry__FDK`).
///
/// Deviation: the decoder has the hybrid input of a single channel; for a second input channel
/// the port reads zeros.
fn spatial_dec_apply_bypass(self_: &mut SpatialDec, num_input_channels: i32) {
    let complex_hyb_bands = (self_.hybrid_bands.max(0) as usize).min(NUM_HYB);
    let hb = complex_hyb_bands;

    let mut rf: i32 = -1;
    let mut lf: i32 = -1;
    let cf: i32 = -1; // Right Front, Left Front, Center Front

    // Determine output channel indices according to tree config
    if self_.tree_config == TREE_212 {
        // 212
        lf = 0;
        rf = 1;
    }

    let in_row = |a: &[[FixpDbl; NUM_HYB]; NUM_IN_CH], i: usize| -> [FixpDbl; NUM_HYB] {
        a.get(i).copied().unwrap_or([0; NUM_HYB])
    };
    let out_idx =
        |c: i32| -> Option<usize> { (c >= 0 && (c as usize) < NUM_OUT_CH).then_some(c as usize) };

    // Note: numInputChannels might not match the tree config !
    let sources: &[(i32, usize)] = match num_input_channels {
        // (cf > 0 is false for the 212 tree)
        1 => &[(lf, 0), (rf, 0)],
        2 => {
            fdk_assert!(lf != -1);
            fdk_assert!(rf != -1);
            &[(lf, 0), (rf, 1)]
        }
        _ => &[],
    };
    for &(c, src) in sources {
        let r = in_row(&self_.hyb_input_real__fdk, src);
        let i = in_row(&self_.hyb_input_imag__fdk, src);
        if let Some(c) = out_idx(c) {
            self_.hyb_output_real_dry__fdk[c][..hb].copy_from_slice(&r[..hb]);
            self_.hyb_output_imag_dry__fdk[c][..complex_hyb_bands]
                .copy_from_slice(&i[..complex_hyb_bands]);
        }
    }

    for ch in 0..(self_.num_output_channels_at.max(0) as usize).min(NUM_OUT_CH) {
        let chi = ch as i32;
        if chi == lf || chi == rf || chi == cf {
            continue; // Skip bypassed channels
        }
        self_.hyb_output_real_dry__fdk[ch][..hb].fill(0);
        self_.hyb_output_imag_dry__fdk[ch][..complex_hyb_bands].fill(0);
    }
}

/// `SpatialDecSetInternalError(self, bypassMode, err)`: sets the internal error and the bypass
/// mode, returns `MPS_OK`.
fn spatial_dec_set_internal_error(
    self_: &mut SpatialDec,
    bypass_mode: &mut i32,
    err: SacdecError,
) -> SacdecError {
    *bypass_mode = 1;

    if self_.err_int == MPS_OK {
        // store internal error before it gets overwritten
        self_.err_int = err;
    }

    MPS_OK
}

/// `SpatialDecApplyParameterSets(self, frame, mode, inData, qmfInDataReal, qmfInDataImag,
/// nSamples, controlFlags, numInputChannels, mapDescr)` (`qmfInData*` are `NULL` in the
/// library and not used).
fn spatial_dec_apply_parameter_sets(
    self_: &mut SpatialDec,
    qd: &mut FdkQmfDomain,
    cfg: &SpatialSpecificConfig,
    frame: &SpatialBsFrame,
    mode: SpatialdecInputMode,
    in_data: PcmMpsInput<'_>,
    time_out: &mut [PcmMps],
    n_samples: u32,
    control_flags: u32,
    num_input_channels: i32,
    map_descr: &FdkChannelMapDescr,
) -> SacdecError {
    let mut err = MPS_OK;

    let mut alpha: FixpSgl = 0;

    let mut prev_slot = self_.prev_time_slot;
    let mut ps = self_.cur_ps;
    let mut ts_io = 0; // i/o dependent slot
    let mut bypass_mode = (control_flags & MPEGS_BYPASSMODE != 0) as i32;

    // Bypass can be triggered by the upmixType, too.
    bypass_mode |= (self_.upmix_type == UPMIXTYPE_BYPASS) as i32;

    // Decode available slots
    // (qmfBands > 0 after a successful FDK_SpatialDecInit(); deviation: 0 processes no slot.)
    let Some(slots) = (n_samples as i32).checked_div(self_.qmf_bands) else {
        return MPS_OK;
    };
    let last =
        fix_min(self_.cur_time_slot.wrapping_add(slots).wrapping_sub(1), self_.time_slots - 1);
    let mut ts = self_.cur_time_slot;
    while ts <= last {
        let curr_slot = param_slot_at(frame, ps);
        err = if curr_slot < ts { MPS_WRONG_PARAMETERSETS } else { MPS_OK };
        if err != MPS_OK {
            err = spatial_dec_set_internal_error(self_, &mut bypass_mode, err);
        }

        // Get new parameter set
        if ts == prev_slot.wrapping_add(1) {
            if bypass_mode == 0 {
                // input: ottCLD, ottICC, ...
                // output: M1param(Real/Imag), M2(Real/Imag)
                err = spatial_dec_calculate_m1_and_m2(self_, ps, frame);
                if err != MPS_OK {
                    err = spatial_dec_set_internal_error(self_, &mut bypass_mode, err);
                }
            }

            if ps == 0 && self_.b_overwrite_m1m2prev != 0 {
                // copy matrix entries of M1/M2 of the first parameter set to the previous
                // matrices (of the last frame). This avoids the interpolation of incompatible
                // values. E.g. for residual bands the coefficients are calculated differently
                // compared to non-residual bands.
                // input: M(1/2)param(Real/Imag); output: M(1/2)param(Real/Imag)Prev
                spatial_dec_buffer_matrices(self_);
                self_.b_overwrite_m1m2prev = 0;
            }

            if bypass_mode == 0 {
                // input: M1param(Real/Imag)(Prev), M2(Real/Imag)(Prev)
                // output: M1param(Real/Imag), M2(Real/Imag)
                spatial_dec_smooth_m1_and_m2(self_, frame, ps);
            }
        }

        if bypass_mode == 0 {
            alpha = fx_dbl2fx_sgl(f_div_norm_noexp(ts - prev_slot, curr_slot - prev_slot));
        }

        match mode {
            INPUTMODE_QMF_SBR => {
                if cfg.syntax_flags & SACDEC_SYNTAX_LD != 0 {
                    self_.b_share_delay_with_sbr = 0; // We got no hybrid delay
                } else {
                    self_.b_share_delay_with_sbr = 1;
                }
                let n =
                    if bypass_mode != 0 { num_input_channels } else { self_.num_input_channels };
                spatial_dec_feed_qmf(self_, qd, ts_io, bypass_mode, n);
            }
            INPUTMODE_TIME => {
                self_.b_share_delay_with_sbr = 0;
                let n =
                    if bypass_mode != 0 { num_input_channels } else { self_.num_input_channels };
                spatial_dec_qmf_analysis(self_, qd, in_data, time_out, ts_io, bypass_mode, n);
            }
            _ => {}
        }

        if cfg.syntax_flags & SACDEC_SYNTAX_USAC != 0 && self_.residual_coding != 0 {
            let ch = 1usize;

            let offset =
                qd.global_conf.n_bands_synthesis as i64 * qd.global_conf.n_qmf_time_slots as i64;

            {
                // inSamples = &inData[ts * nBandsAnalysis] (ts, not ts_io), + ch * offset
                let pos = ts as i64 * qd.global_conf.n_bands_analysis as i64 + ch as i64 * offset;
                let mut in_samples = [0 as PcmMps; 2 * NUM_QMF];
                read_input(in_data, time_out, pos, &mut in_samples);
                {
                    let qd_in = &mut qd.qmf_domain_in[ch];
                    let states = qd_in.p_ana_qmf_states.as_deref_mut().unwrap_or(&mut []);
                    calculate_space_analysis_qmf(
                        Some((&qd_in.fb, states)),
                        &in_samples,
                        &mut self_.qmf_residual_real__fdk[0][0],
                        &mut self_.qmf_residual_imag__fdk[0][0],
                    );
                }

                if !is_two_ch_mode(self_.upmix_type) && bypass_mode == 0 {
                    let headroom = if qd.global_conf.n_bands_analysis == 24
                        && self_.stereo_config_index != 3
                    {
                        (1i32).wrapping_add(self_.sac_in_data_headroom).wrapping_sub(1)
                    } else {
                        self_.sac_in_data_headroom.wrapping_sub(1)
                    };
                    let g = self_.clip_protect_gain__fdk;
                    let n = (self_.qmf_bands.max(0) as usize).min(NUM_QMF);
                    let re = &mut self_.qmf_residual_real__fdk[0][0];
                    for v in re[..n].iter_mut() {
                        *v = f_mult(scale_value_saturate(*v, headroom), g);
                    }
                    let im = &mut self_.qmf_residual_imag__fdk[0][0];
                    for v in im[..n].iter_mut() {
                        *v = f_mult(scale_value_saturate(*v, headroom), g);
                    }
                }
            }
        }

        // input: qmfInput(Real/Imag), qmfResidual(Real/Imag)
        spatial_dec_hybrid_analysis(self_, cfg, ts, num_input_channels);

        if bypass_mode != 0 {
            // input: hybInput(Real/Imag); output: hybOutput(Real/Imag)Dry
            spatial_dec_apply_bypass(self_, num_input_channels);
        } else {
            // !bypassMode
            // input: hybInput(Real/Imag), hybResidual(Real/Imag)
            spatial_dec_create_x(self_);

            // output: w(Real/Imag); the return value is not checked in C
            spatial_dec_apply_m1_create_w_mode212(self_, frame);

            if err != MPS_OK {
                return err;
            }

            let mut apply_m2_config = APPLY_M2_NONE;
            if apply_m2_config == APPLY_M2_NONE {
                apply_m2_config = APPLY_M2;
            }
            if cfg.syntax_flags & (SACDEC_SYNTAX_USAC | SACDEC_SYNTAX_RSVD50) != 0
                && self_.temp_shape_config != 1
                && self_.temp_shape_config != 2
            {
                if self_.phase_coding == 3 {
                    apply_m2_config = APPLY_M2_MODE212_RES_PHASE_CODING;
                } else {
                    apply_m2_config = APPLY_M2_MODE212;
                }
            }

            err = match apply_m2_config {
                APPLY_M2_MODE212 => spatial_dec_apply_m2_mode212(self_, ps, alpha),
                APPLY_M2_MODE212_RES_PHASE_CODING => {
                    spatial_dec_apply_m2_mode212_residuals_plus_phase_coding(self_, cfg, ps, alpha)
                }
                APPLY_M2 => spatial_dec_apply_m2(self_, ps, alpha),
                _ => return MPS_APPLY_M2_ERROR,
            };

            if err != MPS_OK {
                return err;
            }

            if self_.temp_shape_config == 2 && !is_two_ch_mode(self_.upmix_type) {
                // input: reshapeBBEnvState, hybOutput(Real/Imag)(Dry/Wet), hybInput(Real/Imag)
                // output: hybOutput(Real/Imag)Dry
                spatial_dec_reshape_bb_env(self_, frame, ts);
            }

            // Merge parts of the dry and wet QMF buffers.
            if self_.temp_shape_config == 1 && !is_two_ch_mode(self_.upmix_type) {
                let border = (self_.tp_hyb_band_border.max(0) as usize).min(NUM_HYB);
                for ch in 0..(self_.num_output_channels.max(0) as usize).min(NUM_OUT_CH) {
                    for hyb in 0..border {
                        self_.hyb_output_real_dry__fdk[ch][hyb] = f_add_saturate(
                            self_.hyb_output_real_dry__fdk[ch][hyb],
                            self_.hyb_output_real_wet__fdk[ch][hyb],
                        );
                        self_.hyb_output_imag_dry__fdk[ch][hyb] = f_add_saturate(
                            self_.hyb_output_imag_dry__fdk[ch][hyb],
                            self_.hyb_output_imag_wet__fdk[ch][hyb],
                        );
                    } // loop hyb
                } // loop ch
                  // input: hStpDec, hybOutput(Real/Imag)Dry/Wet
                  // output: hStpDec, hybOutput(Real/Imag)Dry
                err = subband_tp_apply(self_, frame);
                if err != MPS_OK {
                    return err;
                }
            }
            // (self->tempShapeConfig == 1)
            else {
                // The wet signal is added to the dry signal in applyM2 if GES and STP are
                // disabled
                if self_.temp_shape_config == 1 || self_.temp_shape_config == 2 {
                    let n_hyb_bands = (self_.hybrid_bands.max(0) as usize).min(NUM_HYB);
                    for ch in 0..(self_.num_output_channels.max(0) as usize).min(NUM_OUT_CH) {
                        for hyb in 0..n_hyb_bands {
                            self_.hyb_output_real_dry__fdk[ch][hyb] = f_add_saturate(
                                self_.hyb_output_real_dry__fdk[ch][hyb],
                                self_.hyb_output_real_wet__fdk[ch][hyb],
                            );
                            self_.hyb_output_imag_dry__fdk[ch][hyb] = f_add_saturate(
                                self_.hyb_output_imag_dry__fdk[ch][hyb],
                                self_.hyb_output_imag_wet__fdk[ch][hyb],
                            );
                        } // loop hyb
                          // for (; hyb < self->hybridBands; hyb++): no band left
                    } // loop ch
                } // ( self->tempShapeConfig == 1 ) || ( self->tempShapeConfig == 2 )
            } // !self->tempShapeConfig == 1
        } // !bypassMode

        if self_.phase_coding == 1 && bypass_mode == 0 {
            // only if bsPhaseCoding == 1 and bsResidualCoding == 0
            // signal the last slot of the set
            spatial_dec_apply_phase(self_, alpha, (ts == curr_slot) as i32);
        }

        // Synthesis Filtering
        // input: hybOutput(Real/Imag)Dry; output: timeOut
        err = spatial_dec_synthesis(self_, qd, cfg, ts_io, time_out, num_input_channels, map_descr);

        if err != MPS_OK {
            return err;
        }

        // Update parameter buffer
        if ts == curr_slot {
            // input: M(1/2)param(Real/Imag); output: M(1/2)param(Real/Imag)Prev
            spatial_dec_buffer_matrices(self_);

            prev_slot = curr_slot;
            ps += 1;
        } // if (ts==currSlot)

        ts += 1;
        ts_io += 1;
    } // ts loop

    // Save parameter states
    self_.prev_time_slot = prev_slot;
    self_.cur_time_slot = ts;
    self_.cur_ps = ps;

    err
}

/// `SpatialDecApplyFrame(self, frame, inputMode, inData, qmfInDataReal, qmfInDataImag,
/// pcmOutBuf, nSamples, pControlFlags, numInputChannels, mapDescr)`: applies the decoded MPEG
/// Surround parameters to the downmix (time or QMF domain) and writes the upmix into
/// `pcm_out_buf` (`qmfInData*` are `NULL` in the library and not used). `cfg` is
/// `self->pConfigCurrent`, `ll_state` the lossless state of the parser.
pub fn spatial_dec_apply_frame(
    self_: &mut SpatialDec,
    qd: &mut FdkQmfDomain,
    cfg: &mut SpatialSpecificConfig,
    frame: &mut SpatialBsFrame,
    ll_state: &mut BsLlState,
    input_mode: SpatialdecInputMode,
    in_data: PcmMpsInput<'_>,
    pcm_out_buf: &mut [PcmMps],
    n_samples: u32,
    p_control_flags: &mut u32,
    mut num_input_channels: i32,
    map_descr: &FdkChannelMapDescr,
) -> SacdecError {
    let mut err = MPS_OK;

    fdk_assert!(self_.sac_in_data_headroom >= 1);

    self_.err_int = err; // Init internal error

    let mut control_flags = *p_control_flags;

    if cfg.syntax_flags & SACDEC_SYNTAX_USAC != 0 && self_.stereo_config_index > 1 {
        // Do not count residual channel as input channel. It is handled seperately.
        num_input_channels = 1;
    }

    // Check if input amount of channels is consistent
    if num_input_channels != self_.num_input_channels {
        control_flags |= MPEGS_CONCEAL;
        if num_input_channels > self_.create_params.max_num_input_channels {
            return MPS_INVALID_PARAMETER;
        }
    }

    // self->timeOut__FDK = pcmOutBuf (a parameter in the port)

    // Determine local function control flags
    let mut f_dec_and_map_frame_data = frame.new_bs_data as i32;

    // (qmfBands > 0 after a successful FDK_SpatialDecInit(); deviation: 0 gives 0 slots.)
    let slots = (n_samples as i32).checked_div(self_.qmf_bands).unwrap_or(0);
    // (fDecAndMapFrameData == 0 assures that conceal flag will not be set for blind mode)
    if (f_dec_and_map_frame_data == 0 && self_.cur_time_slot.wrapping_add(slots) > self_.time_slots)
        || frame.num_parameter_sets == 0
    {
        // New input samples but missing side info
        f_dec_and_map_frame_data = 1;
        control_flags |= MPEGS_CONCEAL;
    }

    if f_dec_and_map_frame_data == 0
        && (param_slot_at(frame, f_max(0, frame.num_parameter_sets as i32 - 1))
            != (self_.time_slots - 1)
            || self_.cur_time_slot > param_slot_at(frame, self_.cur_ps))
    {
        // Detected faulty parameter slot data.
        f_dec_and_map_frame_data = 1;
        control_flags |= MPEGS_CONCEAL;
    }

    // Update concealment state machine
    // (convert from conceal flag to frame ok flag)
    spatial_dec_concealment_update_state(
        &mut self_.conceal_info,
        if control_flags & MPEGS_CONCEAL != 0 { 0 } else { 1 },
    );

    if f_dec_and_map_frame_data != 0 {
        // Reset spatial framing control vars
        frame.new_bs_data = 0;
        self_.prev_time_slot = -1;
        self_.cur_time_slot = 0;
        self_.cur_ps = 0;

        if control_flags & MPEGS_CONCEAL != 0 {
            // Reset frame data to avoid misconfiguration.
            let setup = self_.create_params;
            spatial_dec_clear_frame_data(self_, frame, &setup);
        }

        {
            // input: ...; output: decodeAndMapFrameDATA
            err = spatial_dec_decode_frame(self_, cfg, frame, ll_state);
        }

        if err != MPS_OK {
            // Rescue strategy is to apply bypass mode in order to keep at least the downmix
            // channels continuous.
            control_flags |= MPEGS_CONCEAL;
            if self_.err_int == MPS_OK {
                // store internal error befor it gets overwritten
                self_.err_int = err;
            }
        }
    }

    err = spatial_dec_apply_parameter_sets(
        self_,
        qd,
        cfg,
        frame,
        input_mode,
        in_data,
        pcm_out_buf,
        n_samples,
        control_flags | if err == MPS_OK { 0 } else { MPEGS_BYPASSMODE },
        num_input_channels,
        map_descr,
    );

    *p_control_flags = control_flags;
    err
}
