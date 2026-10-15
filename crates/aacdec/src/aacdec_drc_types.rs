// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACdec/src/aacdec_drc_types.h.

//! Port of `aacdec_drc_types.h`: dynamic range control (DRC) global data types.

use fdk_aac_rs_fdk::common_fix::FixpDbl;

/// `MAX_DRC_THREADS`: heavy compression value is handled just like MPEG DRC data
pub const MAX_DRC_THREADS: i32 = 8 + 1;
/// `MAX_DRC_BANDS`: 2^LEN_DRC_BAND_INCR (LEN_DRC_BAND_INCR = 4)
pub const MAX_DRC_BANDS: i32 = 16;

/// `AACDEC_DRC_PAYLOAD_TYPE`: DRC module global data types
pub type AacdecDrcPayloadType = i32;
/// `UNKNOWN_PAYLOAD`
pub const UNKNOWN_PAYLOAD: AacdecDrcPayloadType = 0;
/// `MPEG_DRC_EXT_DATA`
pub const MPEG_DRC_EXT_DATA: AacdecDrcPayloadType = 1;
/// `DVB_DRC_ANC_DATA`
pub const DVB_DRC_ANC_DATA: AacdecDrcPayloadType = 2;

/// `AACDEC_DRC_PARAMETER_HANDLING`: options for parameter handling / presentation mode
pub type AacdecDrcParameterHandling = i32;
/// `DISABLED_PARAMETER_HANDLING`: DRC parameter handling disabled, all parameters are applied
/// as requested.
pub const DISABLED_PARAMETER_HANDLING: AacdecDrcParameterHandling = -1;
/// `ENABLED_PARAMETER_HANDLING`: apply changes to requested DRC parameters to prevent clipping
pub const ENABLED_PARAMETER_HANDLING: AacdecDrcParameterHandling = 0;
/// `DRC_PRESENTATION_MODE_1`: DRC Presentation mode 1
pub const DRC_PRESENTATION_MODE_1: AacdecDrcParameterHandling = 1;
/// `DRC_PRESENTATION_MODE_2`: DRC Presentation mode 2
pub const DRC_PRESENTATION_MODE_2: AacdecDrcParameterHandling = 2;

/// `CDrcChannelData`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CDrcChannelData {
    /// `expiryCount`
    pub expiry_count: u32,
    /// `numBands`
    pub num_bands: u32,
    /// `bandTop`
    pub band_top: [u16; MAX_DRC_BANDS as usize],
    /// `drcInterpolationScheme`
    pub drc_interpolation_scheme: i16,
    /// `drcValue`
    pub drc_value: [u8; MAX_DRC_BANDS as usize],
    /// `drcDataType`
    pub drc_data_type: i8,
}

/// `CDrcPayload`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CDrcPayload {
    /// `excludedChnsMask`
    pub excluded_chns_mask: u32,
    /// `progRefLevel`
    pub prog_ref_level: i8,
    /// `presMode`: presentation mode: 0 (not indicated), 1, 2, and 3 (reserved).
    pub pres_mode: i8,
    /// `pceInstanceTag`
    pub pce_instance_tag: i8,

    /// `channelData`
    pub channel_data: CDrcChannelData,
}

/// `CDrcParams`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CDrcParams {
    /* DRC parameters: Latest user requests */
    /// `usrCut`
    pub usr_cut: FixpDbl,
    /// `usrBoost`
    pub usr_boost: FixpDbl,
    /// `usrApplyHeavyCompression`
    pub usr_apply_heavy_compression: u8,

    /* DRC parameters: Currently used, possibly changed by aacDecoder_drcParameterHandling */
    /// `cut`: attenuation scale factor
    pub cut: FixpDbl,
    /// `boost`: boost scale factor
    pub boost: FixpDbl,
    /// `targetRefLevel`: target reference level for loudness normalization
    pub target_ref_level: i8,
    /// `applyHeavyCompression`: heavy compression (DVB) flag
    pub apply_heavy_compression: u8,

    /// `expiryFrame`
    pub expiry_frame: u32,
    /// `bsDelayEnable`
    pub bs_delay_enable: u8,

    /// `defaultPresentationMode`
    pub default_presentation_mode: AacdecDrcParameterHandling,
    /// `encoderTargetLevel`
    pub encoder_target_level: u8,
}

/// `CDrcInfo` (`*HANDLE_AAC_DRC`)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CDrcInfo {
    /// `params`: module parameters that can be set by user (via SetParam API function)
    pub params: CDrcParams,

    /// `enable`: switch that controls dynamic range processing
    pub enable: u8,
    /// `digitalNorm`: switch to en-/disable reference level normalization in digital domain
    pub digital_norm: u8,

    /// `update`: flag indicating the change of a user or bitstream parameter which affects
    /// aacDecoder_drcParameterHandling
    pub update: u8,
    /// `numOutChannels`: number of output channels
    pub num_out_channels: i32,
    /// `prevAacNumChannels`: previous number of channels of aac bitstream, used for update flag
    pub prev_aac_num_channels: i32,

    /// `numPayloads`: the number of DRC data payload elements found within frame
    pub num_payloads: u16,
    /// `numThreads`: the number of DRC data threads extracted from the found payload elements
    pub num_threads: u16,
    /// `progRefLevel`: program reference level for all channels
    pub prog_ref_level: i8,
    /// `progRefLevelPresent`: program reference level found in bitstream
    pub prog_ref_level_present: u8,

    /// `prlExpiryCount`: counter that can be used to monitor the life time of the program
    /// reference level.
    pub prl_expiry_count: u32,

    /// `presMode`: presentation mode as defined in ETSI TS 101 154
    pub pres_mode: i8,
    /// `dvbAncDataAvailable`: flag that indicates whether DVB ancillary data is present or not
    pub dvb_anc_data_available: u8,
    /// `dvbAncDataPosition`: used to store the DVB ancillary data payload position in the
    /// bitstream (only one per frame)
    pub dvb_anc_data_position: u32,
    /// `drcPayloadPosition`: used to store the DRC payload positions in the bitstream
    pub drc_payload_position: [u32; MAX_DRC_THREADS as usize],
    /// `applyExtGain`: flag is 1 if extGain has to be applied, otherwise 0.
    pub apply_ext_gain: u8,

    /// `additionalGainPrev`: gain of previous frame to be applied to the time data
    pub additional_gain_prev: FixpDbl,
    /// `additionalGainFilterState`: filter state for the gain smoothing
    pub additional_gain_filter_state: FixpDbl,
    /// `additionalGainFilterState1`: filter state for the gain smoothing
    pub additional_gain_filter_state1: FixpDbl,
}

/// `HANDLE_AAC_DRC`
pub type HandleAacDrc<'a> = &'a mut CDrcInfo;
