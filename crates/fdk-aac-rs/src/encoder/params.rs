// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACenc/include/aacenc_lib.h (AACENC_PARAM and the values it documents),
// libSYS/include/FDK_audio.h (AUDIO_OBJECT_TYPE, CHANNEL_MODE, TRANSPORT_TYPE).

//! Encoder parameters (`AACENC_PARAM`) and their values.
//!
//! Every enum has the C value as its discriminant: [`ParamId`] the parameter ID, the value enums
//! the value that `aacEncoder_SetParam()` receives. [`Param`] combines both; its discriminant is
//! the parameter ID as well.

use fdk_aac_rs_aacenc::aacenc_lib as lib;
use fdk_aac_rs_fdk::audio as au;

/// Parameter IDs of `aacEncoder_SetParam()` and `aacEncoder_GetParam()` (`AACENC_PARAM`).
#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ParamId {
    /// `AACENC_AOT`: audio object type.
    Aot = lib::AACENC_AOT as u32,
    /// `AACENC_BITRATE`: total bitrate in bits/second (CBR).
    Bitrate = lib::AACENC_BITRATE as u32,
    /// `AACENC_BITRATEMODE`: CBR or one of the VBR modes.
    BitrateMode = lib::AACENC_BITRATEMODE as u32,
    /// `AACENC_SAMPLERATE`: input sampling rate.
    SampleRate = lib::AACENC_SAMPLERATE as u32,
    /// `AACENC_SBR_MODE`: SBR for AAC-ELD.
    SbrMode = lib::AACENC_SBR_MODE as u32,
    /// `AACENC_GRANULE_LENGTH`: core frame length.
    GranuleLength = lib::AACENC_GRANULE_LENGTH as u32,
    /// `AACENC_CHANNELMODE`: channel mode.
    ChannelMode = lib::AACENC_CHANNELMODE as u32,
    /// `AACENC_CHANNELORDER`: input channel order.
    ChannelOrder = lib::AACENC_CHANNELORDER as u32,
    /// `AACENC_SBR_RATIO`: downsampled or dual-rate SBR.
    SbrRatio = lib::AACENC_SBR_RATIO as u32,
    /// `AACENC_AFTERBURNER`: afterburner on or off.
    Afterburner = lib::AACENC_AFTERBURNER as u32,
    /// `AACENC_BANDWIDTH`: core audio bandwidth in Hz (0: automatic).
    Bandwidth = lib::AACENC_BANDWIDTH as u32,
    /// `AACENC_PEAK_BITRATE`: peak bitrate in bits/second.
    PeakBitrate = lib::AACENC_PEAK_BITRATE as u32,
    /// `AACENC_TRANSMUX`: transport type.
    Transmux = lib::AACENC_TRANSMUX as u32,
    /// `AACENC_HEADER_PERIOD`: in-band configuration period in frames (0xFF: automatic).
    HeaderPeriod = lib::AACENC_HEADER_PERIOD as u32,
    /// `AACENC_SIGNALING_MODE`: signaling of the extension AOT.
    SignalingMode = lib::AACENC_SIGNALING_MODE as u32,
    /// `AACENC_TPSUBFRAMES`: sub frames per transport frame.
    TpSubframes = lib::AACENC_TPSUBFRAMES as u32,
    /// `AACENC_AUDIOMUXVER`: LATM AudioMuxVersion.
    AudioMuxVersion = lib::AACENC_AUDIOMUXVER as u32,
    /// `AACENC_PROTECTION`: ADTS CRC.
    Protection = lib::AACENC_PROTECTION as u32,
    /// `AACENC_ANCILLARY_BITRATE`: constant ancillary data bitrate in bits/second.
    AncillaryBitrate = lib::AACENC_ANCILLARY_BITRATE as u32,
    /// `AACENC_METADATA_MODE`: metadata embedding.
    MetadataMode = lib::AACENC_METADATA_MODE as u32,
    /// `AACENC_CONTROL_STATE`: the pending reinitialization flags (`AACENC_CTRLFLAGS`).
    ControlState = lib::AACENC_CONTROL_STATE as u32,
    /// `AACENC_NONE`: no parameter.
    None = lib::AACENC_NONE as u32,
}

impl ParamId {
    /// The C parameter ID.
    #[inline]
    pub const fn code(self) -> u32 {
        self as u32
    }
}

/// Audio object types the encoder accepts (`AUDIO_OBJECT_TYPE`, `AACENC_AOT`).
#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Aot {
    /// `AOT_AAC_LC` (2): MPEG-4 AAC Low Complexity.
    AacLc = au::AOT_AAC_LC as u32,
    /// `AOT_SBR` (5): HE-AAC (AAC-LC with SBR).
    HeAac = au::AOT_SBR as u32,
    /// `AOT_ER_AAC_LD` (23): AAC Low Delay.
    AacLd = au::AOT_ER_AAC_LD as u32,
    /// `AOT_PS` (29): HE-AAC v2 (AAC-LC with SBR and PS).
    HeAacV2 = au::AOT_PS as u32,
    /// `AOT_ER_AAC_ELD` (39): AAC Enhanced Low Delay.
    AacEld = au::AOT_ER_AAC_ELD as u32,
    /// `AOT_MP2_AAC_LC` (129): MPEG-2 AAC Low Complexity.
    Mpeg2AacLc = au::AOT_MP2_AAC_LC as u32,
    /// `AOT_MP2_SBR` (132): MPEG-2 HE-AAC.
    Mpeg2HeAac = au::AOT_MP2_SBR as u32,
}

/// Bitrate modes (`AACENC_BITRATEMODE`).
#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BitrateMode {
    /// 0: constant bitrate (`AACENC_BITRATE`).
    Cbr = 0,
    /// 1: VBR, very low bitrate.
    VbrVeryLow = 1,
    /// 2: VBR, low bitrate.
    VbrLow = 2,
    /// 3: VBR, medium bitrate.
    VbrMedium = 3,
    /// 4: VBR, high bitrate.
    VbrHigh = 4,
    /// 5: VBR, very high bitrate.
    VbrVeryHigh = 5,
}

/// A bitrate configuration for [`EncoderBuilder::bitrate`](super::EncoderBuilder::bitrate).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Bitrate {
    /// Constant bitrate in bits/second: `AACENC_BITRATEMODE` 0, then `AACENC_BITRATE`.
    Cbr(u32),
    /// Variable bitrate: `AACENC_BITRATEMODE` with the given mode.
    Vbr(BitrateMode),
}

/// SBR configuration of AAC-ELD (`AACENC_SBR_MODE`).
#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SbrMode {
    /// -1: ELD SBR auto configurator (default).
    Auto = -1,
    /// 0: SBR disabled.
    Disabled = 0,
    /// 1: SBR enabled.
    Enabled = 1,
}

/// SBR sampling rate ratio (`AACENC_SBR_RATIO`).
#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SbrRatio {
    /// 0: the default of the audio object type.
    Default = 0,
    /// 1: downsampled SBR.
    Downsampled = 1,
    /// 2: dual-rate SBR.
    DualRate = 2,
}

/// Channel modes (`CHANNEL_MODE`, `AACENC_CHANNELMODE`).
#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ChannelMode {
    /// `MODE_1` (1): C.
    Mono = au::MODE_1 as u32,
    /// `MODE_2` (2): L+R.
    Stereo = au::MODE_2 as u32,
    /// `MODE_1_2` (3): C, L+R.
    Mode1_2 = au::MODE_1_2 as u32,
    /// `MODE_1_2_1` (4): C, L+R, Rear.
    Mode1_2_1 = au::MODE_1_2_1 as u32,
    /// `MODE_1_2_2` (5): C, L+R, LS+RS.
    Mode1_2_2 = au::MODE_1_2_2 as u32,
    /// `MODE_1_2_2_1` (6): C, L+R, LS+RS, LFE (5.1).
    Mode1_2_2_1 = au::MODE_1_2_2_1 as u32,
    /// `MODE_1_2_2_2_1` (7): C, LC+RC, L+R, LS+RS, LFE.
    Mode1_2_2_2_1 = au::MODE_1_2_2_2_1 as u32,
    /// `MODE_6_1` (11): C, L+R, LS+RS, Crear, LFE.
    Mode6_1 = au::MODE_6_1 as u32,
    /// `MODE_7_1_BACK` (12): C, L+R, LS+RS, Lrear+Rrear, LFE.
    Mode7_1Back = au::MODE_7_1_BACK as u32,
    /// `MODE_7_1_TOP_FRONT` (14): C, L+R, LS+RS, LFE, Ltop+Rtop.
    Mode7_1TopFront = au::MODE_7_1_TOP_FRONT as u32,
    /// `MODE_7_1_REAR_SURROUND` (33): C, L+R, LS+RS, Lrear+Rrear, LFE.
    Mode7_1RearSurround = au::MODE_7_1_REAR_SURROUND as u32,
    /// `MODE_7_1_FRONT_CENTER` (34): C, LC+RC, L+R, LS+RS, LFE.
    Mode7_1FrontCenter = au::MODE_7_1_FRONT_CENTER as u32,
    /// `MODE_212` (128): stereo input coded as ELDv2 212 (MPEG Surround).
    Mode212 = au::MODE_212 as u32,
}

/// Input channel order (`AACENC_CHANNELORDER`).
#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ChannelOrder {
    /// 0: MPEG channel order (5.1: C, L, R, SL, SR, LFE), the default.
    Mpeg = 0,
    /// 1: WAVE channel order (5.1: L, R, C, LFE, SL, SR).
    Wave = 1,
    /// 2: WG4 channel order (5.1: L, R, SL, SR, C, LFE).
    Wg4 = 2,
}

/// Transport types (`TRANSPORT_TYPE`, `AACENC_TRANSMUX`).
#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Transport {
    /// `TT_MP4_RAW` (0): raw access units.
    Raw = au::TT_MP4_RAW as u32,
    /// `TT_MP4_ADIF` (1): ADIF.
    Adif = au::TT_MP4_ADIF as u32,
    /// `TT_MP4_ADTS` (2): ADTS.
    Adts = au::TT_MP4_ADTS as u32,
    /// `TT_MP4_LATM_MCP1` (6): LATM with muxConfigPresent = 1.
    LatmMcp1 = au::TT_MP4_LATM_MCP1 as u32,
    /// `TT_MP4_LATM_MCP0` (7): LATM with muxConfigPresent = 0 (out-of-band StreamMuxConfig).
    LatmMcp0 = au::TT_MP4_LATM_MCP0 as u32,
    /// `TT_MP4_LOAS` (10): LOAS.
    Loas = au::TT_MP4_LOAS as u32,
}

/// Signaling of the extension audio object type (`AACENC_SIGNALING_MODE`).
#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SignalingMode {
    /// 0: implicit backward compatible signaling.
    Implicit = 0,
    /// 1: explicit backward compatible signaling.
    ExplicitBackwardCompatible = 1,
    /// 2: explicit hierarchical signaling.
    ExplicitHierarchical = 2,
}

/// Metadata embedding (`AACENC_METADATA_MODE`).
#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MetadataMode {
    /// 0: no metadata.
    None = 0,
    /// 1: dynamic_range_info.
    DynamicRange = 1,
    /// 2: dynamic_range_info and ancillary_data.
    DynamicRangeAndAncillary = 2,
    /// 3: ancillary_data.
    Ancillary = 3,
}

/// A parameter with its value, for `aacEncoder_SetParam()`.
///
/// The discriminant is the C parameter ID ([`ParamId`]); [`Param::value`] is the `UINT` value
/// the C function receives. Numeric parameters take any `u32`, so values the library rejects
/// (with [`EncoderError::InvalidConfig`](super::EncoderError::InvalidConfig)) can be passed as
/// in C. [`Encoder::set_raw`](super::Encoder::set_raw) takes any ID and value.
#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Param {
    /// `AACENC_AOT`
    Aot(Aot) = ParamId::Aot as u32,
    /// `AACENC_BITRATE` in bits/second.
    Bitrate(u32) = ParamId::Bitrate as u32,
    /// `AACENC_BITRATEMODE`
    BitrateMode(BitrateMode) = ParamId::BitrateMode as u32,
    /// `AACENC_SAMPLERATE` in Hz.
    SampleRate(u32) = ParamId::SampleRate as u32,
    /// `AACENC_SBR_MODE`
    SbrMode(SbrMode) = ParamId::SbrMode as u32,
    /// `AACENC_GRANULE_LENGTH` in samples.
    GranuleLength(u32) = ParamId::GranuleLength as u32,
    /// `AACENC_CHANNELMODE`
    ChannelMode(ChannelMode) = ParamId::ChannelMode as u32,
    /// `AACENC_CHANNELORDER`
    ChannelOrder(ChannelOrder) = ParamId::ChannelOrder as u32,
    /// `AACENC_SBR_RATIO`
    SbrRatio(SbrRatio) = ParamId::SbrRatio as u32,
    /// `AACENC_AFTERBURNER`
    Afterburner(bool) = ParamId::Afterburner as u32,
    /// `AACENC_BANDWIDTH` in Hz (0: automatic).
    Bandwidth(u32) = ParamId::Bandwidth as u32,
    /// `AACENC_PEAK_BITRATE` in bits/second.
    PeakBitrate(u32) = ParamId::PeakBitrate as u32,
    /// `AACENC_TRANSMUX`
    Transmux(Transport) = ParamId::Transmux as u32,
    /// `AACENC_HEADER_PERIOD` in frames (0xFF: automatic).
    HeaderPeriod(u32) = ParamId::HeaderPeriod as u32,
    /// `AACENC_SIGNALING_MODE`
    SignalingMode(SignalingMode) = ParamId::SignalingMode as u32,
    /// `AACENC_TPSUBFRAMES`
    TpSubframes(u32) = ParamId::TpSubframes as u32,
    /// `AACENC_AUDIOMUXVER`
    AudioMuxVersion(u32) = ParamId::AudioMuxVersion as u32,
    /// `AACENC_PROTECTION`
    Protection(bool) = ParamId::Protection as u32,
    /// `AACENC_ANCILLARY_BITRATE` in bits/second.
    AncillaryBitrate(u32) = ParamId::AncillaryBitrate as u32,
    /// `AACENC_METADATA_MODE`
    MetadataMode(MetadataMode) = ParamId::MetadataMode as u32,
    /// `AACENC_CONTROL_STATE`: `AACENC_CTRLFLAGS` bits (see [`ControlFlags`](super::ControlFlags)).
    ControlState(u32) = ParamId::ControlState as u32,
}

impl Param {
    /// The parameter ID.
    pub const fn id(self) -> ParamId {
        match self {
            Param::Aot(_) => ParamId::Aot,
            Param::Bitrate(_) => ParamId::Bitrate,
            Param::BitrateMode(_) => ParamId::BitrateMode,
            Param::SampleRate(_) => ParamId::SampleRate,
            Param::SbrMode(_) => ParamId::SbrMode,
            Param::GranuleLength(_) => ParamId::GranuleLength,
            Param::ChannelMode(_) => ParamId::ChannelMode,
            Param::ChannelOrder(_) => ParamId::ChannelOrder,
            Param::SbrRatio(_) => ParamId::SbrRatio,
            Param::Afterburner(_) => ParamId::Afterburner,
            Param::Bandwidth(_) => ParamId::Bandwidth,
            Param::PeakBitrate(_) => ParamId::PeakBitrate,
            Param::Transmux(_) => ParamId::Transmux,
            Param::HeaderPeriod(_) => ParamId::HeaderPeriod,
            Param::SignalingMode(_) => ParamId::SignalingMode,
            Param::TpSubframes(_) => ParamId::TpSubframes,
            Param::AudioMuxVersion(_) => ParamId::AudioMuxVersion,
            Param::Protection(_) => ParamId::Protection,
            Param::AncillaryBitrate(_) => ParamId::AncillaryBitrate,
            Param::MetadataMode(_) => ParamId::MetadataMode,
            Param::ControlState(_) => ParamId::ControlState,
        }
    }

    /// The `UINT` value `aacEncoder_SetParam()` receives.
    pub const fn value(self) -> u32 {
        match self {
            Param::Aot(v) => v as u32,
            Param::BitrateMode(v) => v as u32,
            // -1 is passed as (UINT)-1, as in C
            Param::SbrMode(v) => v as i32 as u32,
            Param::ChannelMode(v) => v as u32,
            Param::ChannelOrder(v) => v as u32,
            Param::SbrRatio(v) => v as u32,
            Param::Transmux(v) => v as u32,
            Param::SignalingMode(v) => v as u32,
            Param::MetadataMode(v) => v as u32,
            Param::Afterburner(v) | Param::Protection(v) => v as u32,
            Param::Bitrate(v)
            | Param::SampleRate(v)
            | Param::GranuleLength(v)
            | Param::Bandwidth(v)
            | Param::PeakBitrate(v)
            | Param::HeaderPeriod(v)
            | Param::TpSubframes(v)
            | Param::AudioMuxVersion(v)
            | Param::AncillaryBitrate(v)
            | Param::ControlState(v) => v,
        }
    }
}
