// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libSACdec/src/sac_dec_ssc_struct.h.

//! Port of `sac_dec_ssc_struct.h`: the spatial specific config structure.
//!
//! `FDK_SpatialDecCompareSpatialSpecificConfigHeader()` compares two configurations with
//! `FDKmemcmp` over the whole structure, so [`SpatialSpecificConfig`] has the C layout (192
//! bytes on both reference architectures) with the one padding byte as an explicit field
//! ([`SpatialSpecificConfig::_pad0`]). The library never writes that byte: every configuration
//! lives in zeroed memory or is cleared with `FDKmemclear` before it is parsed, and the
//! structure copies of the library copy it unchanged (see
//! [`crate::sac_dec_lib`]). [`SpatialSpecificConfig::as_bytes`] gives the byte image.

use fdk_aac_rs_fdk::audio::AudioObjectType;

/// `MAX_NUM_QMF_BANDS`
pub const MAX_NUM_QMF_BANDS: i32 = 128;
/// `MAX_TIME_SLOTS`
pub const MAX_TIME_SLOTS: i32 = 64;
/// `MAX_INPUT_CHANNELS`
pub const MAX_INPUT_CHANNELS: usize = 1;
/// `MAX_OUTPUT_CHANNELS`: does not restrict the number of output channels exclusively; it also
/// limits the number of bitstream and residual channels.
pub const MAX_OUTPUT_CHANNELS: usize = 2;
/// `MAX_NUM_OTT`
pub const MAX_NUM_OTT: usize = 5;
/// `MAX_NUM_TTT`
pub const MAX_NUM_TTT: usize = 0;
/// `MAX_NUM_EXT_TYPES`
pub const MAX_NUM_EXT_TYPES: usize = 8;
/// `MAX_PARAMETER_BANDS`
pub const MAX_PARAMETER_BANDS: usize = 28;
/// `MAX_PARAMETER_BANDS_LD`
pub const MAX_PARAMETER_BANDS_LD: usize = 23;
/// `MAX_NUM_XCHANNELS`
pub const MAX_NUM_XCHANNELS: usize = 6;
/// `MAX_ARBITRARY_TREE_LEVELS`
pub const MAX_ARBITRARY_TREE_LEVELS: usize = 0;

/// `SPATIALDEC_FREQ_RES`
pub type SpatialdecFreqRes = i32;
pub const SPATIALDEC_FREQ_RES_40: SpatialdecFreqRes = 40;
pub const SPATIALDEC_FREQ_RES_28: SpatialdecFreqRes = 28;
pub const SPATIALDEC_FREQ_RES_23: SpatialdecFreqRes = 23;
pub const SPATIALDEC_FREQ_RES_20: SpatialdecFreqRes = 20;
pub const SPATIALDEC_FREQ_RES_15: SpatialdecFreqRes = 15;
pub const SPATIALDEC_FREQ_RES_14: SpatialdecFreqRes = 14;
pub const SPATIALDEC_FREQ_RES_12: SpatialdecFreqRes = 12;
pub const SPATIALDEC_FREQ_RES_10: SpatialdecFreqRes = 10;
pub const SPATIALDEC_FREQ_RES_9: SpatialdecFreqRes = 9;
pub const SPATIALDEC_FREQ_RES_7: SpatialdecFreqRes = 7;
pub const SPATIALDEC_FREQ_RES_5: SpatialdecFreqRes = 5;
pub const SPATIALDEC_FREQ_RES_4: SpatialdecFreqRes = 4;

/// `SPATIALDEC_QUANT_MODE`
pub type SpatialdecQuantMode = i32;
pub const SPATIALDEC_QUANT_FINE_DEF: SpatialdecQuantMode = 0;
pub const SPATIALDEC_QUANT_EDQ1: SpatialdecQuantMode = 1;
pub const SPATIALDEC_QUANT_EDQ2: SpatialdecQuantMode = 2;
pub const SPATIALDEC_QUANT_RSVD3: SpatialdecQuantMode = 3;
pub const SPATIALDEC_QUANT_RSVD4: SpatialdecQuantMode = 4;
pub const SPATIALDEC_QUANT_RSVD5: SpatialdecQuantMode = 5;
pub const SPATIALDEC_QUANT_RSVD6: SpatialdecQuantMode = 6;
pub const SPATIALDEC_QUANT_RSVD7: SpatialdecQuantMode = 7;

/// `SPATIALDEC_TREE_CONFIG`
pub type SpatialdecTreeConfig = i32;
/// `SPATIALDEC_MODE_RSVD7`: the 212 tree
pub const SPATIALDEC_MODE_RSVD7: SpatialdecTreeConfig = 7;

/// `SPATIALDEC_FIXED_GAINS`
pub type SpatialdecFixedGains = i32;
pub const SPATIALDEC_GAIN_MODE0: SpatialdecFixedGains = 0;
pub const SPATIALDEC_GAIN_RSVD1: SpatialdecFixedGains = 1;
pub const SPATIALDEC_GAIN_RSVD2: SpatialdecFixedGains = 2;
pub const SPATIALDEC_GAIN_RSVD3: SpatialdecFixedGains = 3;
pub const SPATIALDEC_GAIN_RSVD4: SpatialdecFixedGains = 4;
pub const SPATIALDEC_GAIN_RSVD5: SpatialdecFixedGains = 5;
pub const SPATIALDEC_GAIN_RSVD6: SpatialdecFixedGains = 6;
pub const SPATIALDEC_GAIN_RSVD7: SpatialdecFixedGains = 7;
pub const SPATIALDEC_GAIN_RSVD8: SpatialdecFixedGains = 8;
pub const SPATIALDEC_GAIN_RSVD9: SpatialdecFixedGains = 9;
pub const SPATIALDEC_GAIN_RSVD10: SpatialdecFixedGains = 10;
pub const SPATIALDEC_GAIN_RSVD11: SpatialdecFixedGains = 11;
pub const SPATIALDEC_GAIN_RSVD12: SpatialdecFixedGains = 12;
pub const SPATIALDEC_GAIN_RSVD13: SpatialdecFixedGains = 13;
pub const SPATIALDEC_GAIN_RSVD14: SpatialdecFixedGains = 14;
pub const SPATIALDEC_GAIN_RSVD15: SpatialdecFixedGains = 15;

/// `SPATIALDEC_TS_CONF`
pub type SpatialdecTsConf = i32;
pub const SPATIALDEC_TS_TPNOWHITE: SpatialdecTsConf = 0;
pub const SPATIALDEC_TS_TPWHITE: SpatialdecTsConf = 1;
pub const SPATIALDEC_TS_TES: SpatialdecTsConf = 2;
pub const SPATIALDEC_TS_NOTS: SpatialdecTsConf = 3;
pub const SPATIALDEC_TS_RSVD4: SpatialdecTsConf = 4;
pub const SPATIALDEC_TS_RSVD5: SpatialdecTsConf = 5;
pub const SPATIALDEC_TS_RSVD6: SpatialdecTsConf = 6;
pub const SPATIALDEC_TS_RSVD7: SpatialdecTsConf = 7;
pub const SPATIALDEC_TS_RSVD8: SpatialdecTsConf = 8;
pub const SPATIALDEC_TS_RSVD9: SpatialdecTsConf = 9;
pub const SPATIALDEC_TS_RSVD10: SpatialdecTsConf = 10;
pub const SPATIALDEC_TS_RSVD11: SpatialdecTsConf = 11;
pub const SPATIALDEC_TS_RSVD12: SpatialdecTsConf = 12;
pub const SPATIALDEC_TS_RSVD13: SpatialdecTsConf = 13;
pub const SPATIALDEC_TS_RSVD14: SpatialdecTsConf = 14;
pub const SPATIALDEC_TS_RSVD15: SpatialdecTsConf = 15;

/// `SPATIALDEC_DECORR_CONF`
pub type SpatialdecDecorrConf = i32;
pub const SPATIALDEC_DECORR_MODE0: SpatialdecDecorrConf = 0;
pub const SPATIALDEC_DECORR_MODE1: SpatialdecDecorrConf = 1;
pub const SPATIALDEC_DECORR_MODE2: SpatialdecDecorrConf = 2;
pub const SPATIALDEC_DECORR_RSVD3: SpatialdecDecorrConf = 3;
pub const SPATIALDEC_DECORR_RSVD4: SpatialdecDecorrConf = 4;
pub const SPATIALDEC_DECORR_RSVD5: SpatialdecDecorrConf = 5;
pub const SPATIALDEC_DECORR_RSVD6: SpatialdecDecorrConf = 6;
pub const SPATIALDEC_DECORR_RSVD7: SpatialdecDecorrConf = 7;
pub const SPATIALDEC_DECORR_RSVD8: SpatialdecDecorrConf = 8;
pub const SPATIALDEC_DECORR_RSVD9: SpatialdecDecorrConf = 9;
pub const SPATIALDEC_DECORR_RSVD10: SpatialdecDecorrConf = 10;
pub const SPATIALDEC_DECORR_RSVD11: SpatialdecDecorrConf = 11;
pub const SPATIALDEC_DECORR_RSVD12: SpatialdecDecorrConf = 12;
pub const SPATIALDEC_DECORR_RSVD13: SpatialdecDecorrConf = 13;
pub const SPATIALDEC_DECORR_RSVD14: SpatialdecDecorrConf = 14;
pub const SPATIALDEC_DECORR_RSVD15: SpatialdecDecorrConf = 15;

/// `SPATIALDEC_OTT_CONF`
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SpatialdecOttConf {
    /// `nOttBands`
    pub n_ott_bands: i32,
}

/// `SPATIALDEC_RESIDUAL_CONF`
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SpatialdecResidualConf {
    /// `bResidualPresent`
    pub b_residual_present: i32,
    /// `nResidualBands`
    pub n_residual_bands: i32,
}

/// `SPATIAL_SPECIFIC_CONFIG`, with the C layout (see the [module documentation](self)).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SpatialSpecificConfig {
    /// `syntaxFlags`
    pub syntax_flags: u32,
    /// `samplingFreq`
    pub sampling_freq: i32,
    /// `nTimeSlots`
    pub n_time_slots: i32,
    /// `freqRes`
    pub freq_res: SpatialdecFreqRes,
    /// `treeConfig`
    pub tree_config: SpatialdecTreeConfig,
    /// `quantMode`
    pub quant_mode: SpatialdecQuantMode,
    /// `bArbitraryDownmix`
    pub b_arbitrary_downmix: i32,
    /// `bResidualCoding`
    pub b_residual_coding: i32,
    /// `bsFixedGainDMX`
    pub bs_fixed_gain_dmx: SpatialdecFixedGains,
    /// `tempShapeConfig`
    pub temp_shape_config: SpatialdecTsConf,
    /// `decorrConfig`
    pub decorr_config: SpatialdecDecorrConf,
    /// `nInputChannels`: derived from treeConfig
    pub n_input_channels: i32,
    /// `nOutputChannels`: derived from treeConfig
    pub n_output_channels: i32,
    /// `nOttBoxes`: derived from treeConfig
    pub n_ott_boxes: i32,
    /// `OttConfig[MAX_NUM_OTT]`: dimension nOttBoxes
    pub ott_config: [SpatialdecOttConf; MAX_NUM_OTT],
    /// `nTttBoxes`: derived from treeConfig
    pub n_ttt_boxes: i32,
    /// `ResidualConfig[MAX_NUM_OTT + MAX_NUM_TTT]`: dimension (nOttBoxes + nTttBoxes)
    pub residual_config: [SpatialdecResidualConf; MAX_NUM_OTT + MAX_NUM_TTT],
    /// `sacExtCnt`
    pub sac_ext_cnt: i32,
    /// `sacExtType[MAX_NUM_EXT_TYPES]`
    pub sac_ext_type: [i32; MAX_NUM_EXT_TYPES],
    /// `envQuantMode`
    pub env_quant_mode: i32,
    /// `coreCodec`
    pub core_codec: AudioObjectType,
    /// `stereoConfigIndex`
    pub stereo_config_index: u8,
    /// `coreSbrFrameLengthIndex`: Table 70 in ISO/IEC FDIS 23003-3:2011
    pub core_sbr_frame_length_index: u8,
    /// `bsHighRateMode`
    pub bs_high_rate_mode: u8,
    /// `bsDecorrType`
    pub bs_decorr_type: u8,
    /// `bsPseudoLr`
    pub bs_pseudo_lr: u8,
    /// `bsPhaseCoding`
    pub bs_phase_coding: u8,
    /// `bsOttBandsPhasePresent`
    pub bs_ott_bands_phase_present: u8,
    /// The padding byte before `bsOttBandsPhase` (compared by `FDKmemcmp`; always 0).
    pub _pad0: u8,
    /// `bsOttBandsPhase`
    pub bs_ott_bands_phase: i32,
    /// `ottCLDdefault[MAX_NUM_OTT]`
    pub ott_cld_default: [i8; MAX_NUM_OTT],
    /// `numOttBandsIPD`
    pub num_ott_bands_ipd: u8,
    /// `bitstreamOttBands[MAX_NUM_OTT]`
    pub bitstream_ott_bands: [u8; MAX_NUM_OTT],
    /// `numOttBands[MAX_NUM_OTT]`
    pub num_ott_bands: [u8; MAX_NUM_OTT],
}

/// `sizeof(SPATIAL_SPECIFIC_CONFIG)`
pub const SPATIAL_SPECIFIC_CONFIG_SIZE: usize = 192;

const _: () =
    assert!(core::mem::size_of::<SpatialSpecificConfig>() == SPATIAL_SPECIFIC_CONFIG_SIZE);

impl SpatialSpecificConfig {
    /// The byte image of the structure (what `FDKmemcmp` compares).
    pub fn as_bytes(&self) -> &[u8; SPATIAL_SPECIFIC_CONFIG_SIZE] {
        // SAFETY: the structure is `repr(C)` without implicit padding (the only padding byte is
        // the explicit field `_pad0`, checked by the size assertion and the layout test), so
        // all of its 192 bytes are initialized.
        unsafe { &*(self as *const Self as *const [u8; SPATIAL_SPECIFIC_CONFIG_SIZE]) }
    }

    /// The structure from its byte image (any byte values are valid: all fields are integers).
    pub fn from_bytes(b: &[u8; SPATIAL_SPECIFIC_CONFIG_SIZE]) -> Self {
        let mut s = Self::default();
        // SAFETY: as in `as_bytes`; every bit pattern is a valid value of the integer fields.
        unsafe {
            core::ptr::copy_nonoverlapping(
                b.as_ptr(),
                &mut s as *mut Self as *mut u8,
                SPATIAL_SPECIFIC_CONFIG_SIZE,
            )
        };
        s
    }
}
