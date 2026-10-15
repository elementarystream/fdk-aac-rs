// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACdec/include/aacdecoder_lib.h (AACDEC_PARAM and the values it documents),
// libSYS/include/FDK_audio.h (TRANSPORT_TYPE).

//! Decoder parameters (`AACDEC_PARAM`) and their values.
//!
//! Every enum has the C value as its discriminant: [`ParamId`] the parameter ID, the value enums
//! the value that `aacDecoder_SetParam()` receives. [`DecoderParam`] combines both; its
//! discriminant is the parameter ID as well.

use fdk_aac_rs_aacdec::aacdecoder_lib as lib;
use fdk_aac_rs_fdk::audio as au;

/// Transport types the decoder reads (`TRANSPORT_TYPE`, `transportFmt` of `aacDecoder_Open()`).
#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Transport {
    /// `TT_MP4_RAW` (0): raw access units; the configuration comes from
    /// [`Decoder::config_raw`](super::Decoder::config_raw).
    Raw = au::TT_MP4_RAW,
    /// `TT_MP4_ADIF` (1): ADIF.
    Adif = au::TT_MP4_ADIF,
    /// `TT_MP4_ADTS` (2): ADTS.
    Adts = au::TT_MP4_ADTS,
    /// `TT_MP4_LATM_MCP1` (6): LATM with muxConfigPresent = 1.
    LatmMcp1 = au::TT_MP4_LATM_MCP1,
    /// `TT_MP4_LATM_MCP0` (7): LATM with muxConfigPresent = 0 (out-of-band StreamMuxConfig).
    LatmMcp0 = au::TT_MP4_LATM_MCP0,
    /// `TT_MP4_LOAS` (10): LOAS.
    Loas = au::TT_MP4_LOAS,
    /// `TT_DRM` (12): DRM (Digital Radio Mondiale).
    Drm = au::TT_DRM,
}

/// Parameter IDs of `aacDecoder_SetParam()` (`AACDEC_PARAM`).
#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ParamId {
    /// `AAC_PCM_DUAL_CHANNEL_OUTPUT_MODE`: processing of two-channel signals.
    DualChannelOutputMode = lib::AAC_PCM_DUAL_CHANNEL_OUTPUT_MODE,
    /// `AAC_PCM_OUTPUT_CHANNEL_MAPPING`: output channel order.
    OutputChannelMapping = lib::AAC_PCM_OUTPUT_CHANNEL_MAPPING,
    /// `AAC_PCM_LIMITER_ENABLE`: signal level limiting.
    LimiterEnable = lib::AAC_PCM_LIMITER_ENABLE,
    /// `AAC_PCM_LIMITER_ATTACK_TIME`: limiter attack time in ms.
    LimiterAttackTime = lib::AAC_PCM_LIMITER_ATTACK_TIME,
    /// `AAC_PCM_LIMITER_RELEAS_TIME`: limiter release time in ms.
    LimiterReleaseTime = lib::AAC_PCM_LIMITER_RELEAS_TIME,
    /// `AAC_PCM_MIN_OUTPUT_CHANNELS`: minimum number of output channels.
    MinOutputChannels = lib::AAC_PCM_MIN_OUTPUT_CHANNELS,
    /// `AAC_PCM_MAX_OUTPUT_CHANNELS`: maximum number of output channels (downmix).
    MaxOutputChannels = lib::AAC_PCM_MAX_OUTPUT_CHANNELS,
    /// `AAC_METADATA_PROFILE`: metadata (downmix) profile.
    MetadataProfile = lib::AAC_METADATA_PROFILE,
    /// `AAC_METADATA_EXPIRY_TIME`: metadata expiry time in ms.
    MetadataExpiryTime = lib::AAC_METADATA_EXPIRY_TIME,
    /// `AAC_CONCEAL_METHOD`: error concealment method.
    ConcealMethod = lib::AAC_CONCEAL_METHOD,
    /// `AAC_DRC_BOOST_FACTOR`: DRC boost scaling (0..127).
    DrcBoostFactor = lib::AAC_DRC_BOOST_FACTOR,
    /// `AAC_DRC_ATTENUATION_FACTOR`: DRC attenuation scaling (0..127).
    DrcAttenuationFactor = lib::AAC_DRC_ATTENUATION_FACTOR,
    /// `AAC_DRC_REFERENCE_LEVEL`: target reference level in steps of -0.25 dB.
    DrcReferenceLevel = lib::AAC_DRC_REFERENCE_LEVEL,
    /// `AAC_DRC_HEAVY_COMPRESSION`: DVB heavy compression (RF mode).
    DrcHeavyCompression = lib::AAC_DRC_HEAVY_COMPRESSION,
    /// `AAC_DRC_DEFAULT_PRESENTATION_MODE`: DRC parameter handling.
    DrcDefaultPresentationMode = lib::AAC_DRC_DEFAULT_PRESENTATION_MODE,
    /// `AAC_DRC_ENC_TARGET_LEVEL`: encoder target level in steps of -0.25 dB.
    DrcEncTargetLevel = lib::AAC_DRC_ENC_TARGET_LEVEL,
    /// `AAC_UNIDRC_SET_EFFECT`: requested MPEG-D DRC effect type.
    UniDrcSetEffect = lib::AAC_UNIDRC_SET_EFFECT,
    /// `AAC_UNIDRC_ALBUM_MODE`: MPEG-D DRC album mode.
    UniDrcAlbumMode = lib::AAC_UNIDRC_ALBUM_MODE,
    /// `AAC_QMF_LOWPOWER`: QMF bank processing mode.
    QmfLowPower = lib::AAC_QMF_LOWPOWER,
    /// `AAC_TPDEC_CLEAR_BUFFER`: clear the transport decoder's bitstream buffer.
    TpdecClearBuffer = lib::AAC_TPDEC_CLEAR_BUFFER,
}

impl ParamId {
    /// The C parameter ID.
    #[inline]
    pub const fn code(self) -> i32 {
        self as i32
    }
}

/// Values of [`DecoderParam::DualChannelOutputMode`].
#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DualChannelMode {
    /// 0: leave both signals as they are (default).
    Stereo = 0,
    /// 1: dual mono output from channel 1.
    Channel1 = 1,
    /// 2: dual mono output from channel 2.
    Channel2 = 2,
    /// 3: dual mono output by mixing both channels (L' = R' = 0.5 Ch1 + 0.5 Ch2).
    Mix = 3,
}

/// Values of [`DecoderParam::OutputChannelMapping`].
#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ChannelMapping {
    /// 0: MPEG PCE style order.
    Mpeg = 0,
    /// 1: WAV file channel order (default).
    Wav = 1,
}

/// Values of [`DecoderParam::Limiter`].
#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LimiterMode {
    /// -1: automatic (enabled for all non-low-delay configurations, the default).
    Auto = -1,
    /// 0: disabled.
    Off = 0,
    /// 1: always enabled.
    On = 1,
}

impl From<bool> for LimiterMode {
    fn from(on: bool) -> Self {
        if on {
            LimiterMode::On
        } else {
            LimiterMode::Off
        }
    }
}

/// Values of [`DecoderParam::MetadataProfile`] (`AAC_MD_PROFILE`).
#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MetadataProfile {
    /// `AAC_MD_PROFILE_MPEG_STANDARD` (0): advanced downmix metadata only.
    MpegStandard = lib::AAC_MD_PROFILE_MPEG_STANDARD,
    /// `AAC_MD_PROFILE_MPEG_LEGACY` (1): falls back to the PCE matrix mixdown.
    MpegLegacy = lib::AAC_MD_PROFILE_MPEG_LEGACY,
    /// `AAC_MD_PROFILE_MPEG_LEGACY_PRIO` (2): the PCE matrix mixdown first.
    MpegLegacyPrio = lib::AAC_MD_PROFILE_MPEG_LEGACY_PRIO,
    /// `AAC_MD_PROFILE_ARIB_JAPAN` (3): ARIB STD-B21.
    AribJapan = lib::AAC_MD_PROFILE_ARIB_JAPAN,
}

/// Values of [`DecoderParam::ConcealMethod`].
#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ConcealMethod {
    /// 0: spectral muting.
    SpectralMuting = 0,
    /// 1: noise substitution.
    NoiseSubstitution = 1,
    /// 2: energy interpolation (one frame of additional delay; some AOTs only).
    EnergyInterpolation = 2,
}

/// Values of [`DecoderParam::DrcDefaultPresentationMode`]
/// (`AAC_DRC_DEFAULT_PRESENTATION_MODE_OPTIONS`).
#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DrcPresentationMode {
    /// `AAC_DRC_PARAMETER_HANDLING_DISABLED` (-1): parameters applied as requested (default).
    ParameterHandlingDisabled = lib::AAC_DRC_PARAMETER_HANDLING_DISABLED,
    /// `AAC_DRC_PARAMETER_HANDLING_ENABLED` (0): parameters adjusted to prevent clipping.
    ParameterHandlingEnabled = lib::AAC_DRC_PARAMETER_HANDLING_ENABLED,
    /// `AAC_DRC_PRESENTATION_MODE_1_DEFAULT` (1): presentation mode 1 by default.
    Mode1 = lib::AAC_DRC_PRESENTATION_MODE_1_DEFAULT,
    /// `AAC_DRC_PRESENTATION_MODE_2_DEFAULT` (2): presentation mode 2 by default.
    Mode2 = lib::AAC_DRC_PRESENTATION_MODE_2_DEFAULT,
}

/// Values of [`DecoderParam::UniDrcSetEffect`]: MPEG-D DRC effect types.
#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DrcEffect {
    /// -1: DRC off.
    Off = -1,
    /// 0: none (default); DRC only to prevent clipping.
    None = 0,
    /// 1: late night.
    LateNight = 1,
    /// 2: noisy environment.
    NoisyEnvironment = 2,
    /// 3: limited playback range.
    LimitedPlaybackRange = 3,
    /// 4: low playback level.
    LowPlaybackLevel = 4,
    /// 5: dialog enhancement.
    DialogEnhancement = 5,
    /// 6: general compression.
    GeneralCompression = 6,
}

/// Values of [`DecoderParam::QmfLowPower`].
#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum QmfMode {
    /// -1: internal default.
    Default = -1,
    /// 0: complex QMF.
    Complex = 0,
    /// 1: real (low power) QMF.
    LowPower = 1,
}

/// A parameter with its value, for `aacDecoder_SetParam()`.
///
/// The discriminant is the C parameter ID ([`ParamId`]); [`DecoderParam::value`] is the `INT`
/// value the C function receives. Numeric parameters take any `i32`, so values the library
/// rejects (with [`DecoderError::SetParamFail`](super::DecoderError::SetParamFail)) can be
/// passed as in C. [`Decoder::set_raw`](super::Decoder::set_raw) takes any ID and value.
#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DecoderParam {
    /// `AAC_PCM_DUAL_CHANNEL_OUTPUT_MODE`
    DualChannelOutputMode(DualChannelMode) = ParamId::DualChannelOutputMode as i32,
    /// `AAC_PCM_OUTPUT_CHANNEL_MAPPING`
    OutputChannelMapping(ChannelMapping) = ParamId::OutputChannelMapping as i32,
    /// `AAC_PCM_LIMITER_ENABLE`
    Limiter(LimiterMode) = ParamId::LimiterEnable as i32,
    /// `AAC_PCM_LIMITER_ATTACK_TIME` in ms.
    LimiterAttackTime(i32) = ParamId::LimiterAttackTime as i32,
    /// `AAC_PCM_LIMITER_RELEAS_TIME` in ms.
    LimiterReleaseTime(i32) = ParamId::LimiterReleaseTime as i32,
    /// `AAC_PCM_MIN_OUTPUT_CHANNELS` (-1 or 0: disabled).
    MinOutputChannels(i32) = ParamId::MinOutputChannels as i32,
    /// `AAC_PCM_MAX_OUTPUT_CHANNELS` (-1 or 0: disabled).
    MaxOutputChannels(i32) = ParamId::MaxOutputChannels as i32,
    /// `AAC_METADATA_PROFILE`
    MetadataProfile(MetadataProfile) = ParamId::MetadataProfile as i32,
    /// `AAC_METADATA_EXPIRY_TIME` in ms.
    MetadataExpiryTime(i32) = ParamId::MetadataExpiryTime as i32,
    /// `AAC_CONCEAL_METHOD`
    ConcealMethod(ConcealMethod) = ParamId::ConcealMethod as i32,
    /// `AAC_DRC_BOOST_FACTOR` (0..127).
    DrcBoostFactor(i32) = ParamId::DrcBoostFactor as i32,
    /// `AAC_DRC_ATTENUATION_FACTOR` (0..127).
    DrcAttenuationFactor(i32) = ParamId::DrcAttenuationFactor as i32,
    /// `AAC_DRC_REFERENCE_LEVEL` (40..127, negative: off).
    DrcReferenceLevel(i32) = ParamId::DrcReferenceLevel as i32,
    /// `AAC_DRC_HEAVY_COMPRESSION`
    DrcHeavyCompression(bool) = ParamId::DrcHeavyCompression as i32,
    /// `AAC_DRC_DEFAULT_PRESENTATION_MODE`
    DrcDefaultPresentationMode(DrcPresentationMode) = ParamId::DrcDefaultPresentationMode as i32,
    /// `AAC_DRC_ENC_TARGET_LEVEL` (0..127).
    DrcEncTargetLevel(i32) = ParamId::DrcEncTargetLevel as i32,
    /// `AAC_UNIDRC_SET_EFFECT`
    UniDrcSetEffect(DrcEffect) = ParamId::UniDrcSetEffect as i32,
    /// `AAC_UNIDRC_ALBUM_MODE`
    UniDrcAlbumMode(bool) = ParamId::UniDrcAlbumMode as i32,
    /// `AAC_QMF_LOWPOWER`
    QmfLowPower(QmfMode) = ParamId::QmfLowPower as i32,
    /// `AAC_TPDEC_CLEAR_BUFFER` with the value 1: discard the buffered bitstream.
    TpdecClearBuffer = ParamId::TpdecClearBuffer as i32,
}

impl DecoderParam {
    /// The parameter ID.
    pub const fn id(self) -> ParamId {
        match self {
            DecoderParam::DualChannelOutputMode(_) => ParamId::DualChannelOutputMode,
            DecoderParam::OutputChannelMapping(_) => ParamId::OutputChannelMapping,
            DecoderParam::Limiter(_) => ParamId::LimiterEnable,
            DecoderParam::LimiterAttackTime(_) => ParamId::LimiterAttackTime,
            DecoderParam::LimiterReleaseTime(_) => ParamId::LimiterReleaseTime,
            DecoderParam::MinOutputChannels(_) => ParamId::MinOutputChannels,
            DecoderParam::MaxOutputChannels(_) => ParamId::MaxOutputChannels,
            DecoderParam::MetadataProfile(_) => ParamId::MetadataProfile,
            DecoderParam::MetadataExpiryTime(_) => ParamId::MetadataExpiryTime,
            DecoderParam::ConcealMethod(_) => ParamId::ConcealMethod,
            DecoderParam::DrcBoostFactor(_) => ParamId::DrcBoostFactor,
            DecoderParam::DrcAttenuationFactor(_) => ParamId::DrcAttenuationFactor,
            DecoderParam::DrcReferenceLevel(_) => ParamId::DrcReferenceLevel,
            DecoderParam::DrcHeavyCompression(_) => ParamId::DrcHeavyCompression,
            DecoderParam::DrcDefaultPresentationMode(_) => ParamId::DrcDefaultPresentationMode,
            DecoderParam::DrcEncTargetLevel(_) => ParamId::DrcEncTargetLevel,
            DecoderParam::UniDrcSetEffect(_) => ParamId::UniDrcSetEffect,
            DecoderParam::UniDrcAlbumMode(_) => ParamId::UniDrcAlbumMode,
            DecoderParam::QmfLowPower(_) => ParamId::QmfLowPower,
            DecoderParam::TpdecClearBuffer => ParamId::TpdecClearBuffer,
        }
    }

    /// The `INT` value `aacDecoder_SetParam()` receives.
    pub const fn value(self) -> i32 {
        match self {
            DecoderParam::DualChannelOutputMode(v) => v as i32,
            DecoderParam::OutputChannelMapping(v) => v as i32,
            DecoderParam::Limiter(v) => v as i32,
            DecoderParam::MetadataProfile(v) => v as i32,
            DecoderParam::ConcealMethod(v) => v as i32,
            DecoderParam::DrcDefaultPresentationMode(v) => v as i32,
            DecoderParam::UniDrcSetEffect(v) => v as i32,
            DecoderParam::QmfLowPower(v) => v as i32,
            DecoderParam::DrcHeavyCompression(v) | DecoderParam::UniDrcAlbumMode(v) => v as i32,
            DecoderParam::LimiterAttackTime(v)
            | DecoderParam::LimiterReleaseTime(v)
            | DecoderParam::MinOutputChannels(v)
            | DecoderParam::MaxOutputChannels(v)
            | DecoderParam::MetadataExpiryTime(v)
            | DecoderParam::DrcBoostFactor(v)
            | DecoderParam::DrcAttenuationFactor(v)
            | DecoderParam::DrcReferenceLevel(v)
            | DecoderParam::DrcEncTargetLevel(v) => v,
            DecoderParam::TpdecClearBuffer => 1,
        }
    }
}
