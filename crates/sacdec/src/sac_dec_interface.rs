// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libSACdec/src/sac_dec_interface.h.

//! Port of `sac_dec_interface.h`: the interface types of the spatial decoder core. The functions
//! declared there are defined in [`crate::sac_dec`] and [`crate::sac_bitdec`].

pub use crate::sac_dec_errorcodes::*;
pub use crate::sac_dec_ssc_struct::*;

/// `CFG_LEVEL`: baseline MPEG Surround profile level.
pub type CfgLevel = i32;
/// `DECODER_LEVEL_0`: dummy level; 212 only
pub const DECODER_LEVEL_0: CfgLevel = 0;
/// `DECODER_LEVEL_6`: no support
pub const DECODER_LEVEL_6: CfgLevel = 6;

/// `CFG_RESTRICTION`: number of output channels restriction.
pub type CfgRestriction = i32;
/// `OUTPUT_CHANNELS_DEFAULT`: default configuration depending on decoder level
pub const OUTPUT_CHANNELS_DEFAULT: CfgRestriction = 0;
/// `OUTPUT_CHANNELS_2_0`: limitation to stereo output
pub const OUTPUT_CHANNELS_2_0: CfgRestriction = 1;
/// `OUTPUT_CHANNELS_5_1`: limitation to 5.1 output
pub const OUTPUT_CHANNELS_5_1: CfgRestriction = 2;

/// `CFG_EXTENT`: supported decoder mode.
pub type CfgExtent = i32;
/// `EXT_HQ_ONLY`: high quality processing only
pub const EXT_HQ_ONLY: CfgExtent = 0;
/// `EXT_LP_ONLY`: low power processing only
pub const EXT_LP_ONLY: CfgExtent = 1;
/// `EXT_HQ_AND_LP`: support both HQ and LP processing
pub const EXT_HQ_AND_LP: CfgExtent = 2;

/// `CFG_BINAURAL`: supported binaural mode.
pub type CfgBinaural = i32;
/// `BINAURAL_NONE`: no binaural processing supported
pub const BINAURAL_NONE: CfgBinaural = -1;

/// `SPATIAL_DEC_CONFIG`: the parameters of the decoder open function.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SpatialDecConfig {
    /// `decoderLevel`
    pub decoder_level: CfgLevel,
    /// `decoderMode`
    pub decoder_mode: CfgExtent,
    /// `maxNumOutputChannels`
    pub max_num_output_channels: CfgRestriction,
    /// `binauralMode`
    pub binaural_mode: CfgBinaural,
}

/// `SPATIALDEC_INPUT_MODE`
pub type SpatialdecInputMode = i32;
/// `INPUTMODE_QMF`
pub const INPUTMODE_QMF: SpatialdecInputMode = 1000;
/// `INPUTMODE_QMF_SBR`
pub const INPUTMODE_QMF_SBR: SpatialdecInputMode = 1001;
/// `INPUTMODE_TIME`
pub const INPUTMODE_TIME: SpatialdecInputMode = 1002;

/// `SPATIAL_DEC_UPMIX_TYPE`
pub type SpatialDecUpmixType = i32;
/// `UPMIX_TYPE_BYPASS`: bypass the downmix channels from the core decoder
pub const UPMIX_TYPE_BYPASS: SpatialDecUpmixType = -1;
/// `UPMIX_TYPE_NORMAL`: multi channel output
pub const UPMIX_TYPE_NORMAL: SpatialDecUpmixType = 0;

/// `SPATIALDEC_PARAM`: dynamic decoder parameters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SpatialdecParam {
    /// `outputMode`
    pub output_mode: u8,
    /// `blindEnable`
    pub blind_enable: u8,
    /// `bypassMode`
    pub bypass_mode: u8,
    /// `concealMethod`
    pub conceal_method: u8,
    /// `concealNumKeepFrames`
    pub conceal_num_keep_frames: u32,
    /// `concealFadeOutSlopeLength`
    pub conceal_fade_out_slope_length: u32,
    /// `concealFadeInSlopeLength`
    pub conceal_fade_in_slope_length: u32,
    /// `concealNumReleaseFrames`
    pub conceal_num_release_frames: u32,
}

/// `MPEGS_INIT_CTRL_FLAGS`: flags which control the initialization.
pub type MpegsInitCtrlFlags = u32;
/// `MPEGS_INIT_NONE`
pub const MPEGS_INIT_NONE: MpegsInitCtrlFlags = 0x00000000;
/// `MPEGS_INIT_CONFIG`: configuration change due to SSC value changes
pub const MPEGS_INIT_CONFIG: MpegsInitCtrlFlags = 0x00000010;
/// `MPEGS_INIT_STATES_ANA_QMF_FILTER`
pub const MPEGS_INIT_STATES_ANA_QMF_FILTER: MpegsInitCtrlFlags = 0x00000100;
/// `MPEGS_INIT_STATES_SYN_QMF_FILTER`
pub const MPEGS_INIT_STATES_SYN_QMF_FILTER: MpegsInitCtrlFlags = 0x00000200;
/// `MPEGS_INIT_STATES_ANA_HYB_FILTER`
pub const MPEGS_INIT_STATES_ANA_HYB_FILTER: MpegsInitCtrlFlags = 0x00000400;
/// `MPEGS_INIT_STATES_DECORRELATOR`
pub const MPEGS_INIT_STATES_DECORRELATOR: MpegsInitCtrlFlags = 0x00000800;
/// `MPEGS_INIT_STATES_M1M2`
pub const MPEGS_INIT_STATES_M1M2: MpegsInitCtrlFlags = 0x00002000;
/// `MPEGS_INIT_STATES_GES`
pub const MPEGS_INIT_STATES_GES: MpegsInitCtrlFlags = 0x00004000;
/// `MPEGS_INIT_STATES_REVERB`
pub const MPEGS_INIT_STATES_REVERB: MpegsInitCtrlFlags = 0x00008000;
/// `MPEGS_INIT_STATES_PARAM`
pub const MPEGS_INIT_STATES_PARAM: MpegsInitCtrlFlags = 0x00020000;
/// `MPEGS_INIT_STATES_ERROR_CONCEALMENT`
pub const MPEGS_INIT_STATES_ERROR_CONCEALMENT: MpegsInitCtrlFlags = 0x00080000;
/// `MPEGS_INIT_PARAMS_ERROR_CONCEALMENT`
pub const MPEGS_INIT_PARAMS_ERROR_CONCEALMENT: MpegsInitCtrlFlags = 0x00200000;

/// `MASK_MPEGS_INIT_ALL_STATES`
pub const MASK_MPEGS_INIT_ALL_STATES: u32 = 0x000FFF00;
/// `MASK_MPEGS_INIT_ALL_PARAMS`
pub const MASK_MPEGS_INIT_ALL_PARAMS: u32 = 0x00F00000;

/// `MEM_REQUIREMENTS`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MemRequirements {
    /// `sizePersistent`: persistent memory
    pub size_persistent: u32,
    /// `sizeFastPersistent`: fast persistent memory
    pub size_fast_persistent: u32,
}

/// `PCM_MPS`: the sample type of the MPEG Surround decoder (`LONG`).
pub type PcmMps = i32;
/// `PCM_MPSF`
pub type PcmMpsf = fdk_aac_rs_fdk::common_fix::FixpDbl;

/// `FIXP_DBL2PCM_MPS(x)`
#[inline(always)]
pub const fn fixp_dbl2pcm_mps(x: fdk_aac_rs_fdk::common_fix::FixpDbl) -> PcmMps {
    x
}
