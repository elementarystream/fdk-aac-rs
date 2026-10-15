// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libSBRenc/include/sbr_encoder.h, libSBRenc/src/sbr_encoder.cpp.

//! Port of `sbr_encoder.h` and `sbr_encoder.cpp`: the SBR encoder API (the functions of
//! `sbr_encoder.cpp` are in the child module `api`, re-exported here).
//!
//! # Notes for the AAC encoder glue
//!
//! The API keeps the parameters of the C functions; pointers become references:
//!
//! - `HANDLE_SBR_ENCODER *` is `&mut Option<Box<SbrEncoder>>` ([`sbr_encoder_open`],
//!   [`sbr_encoder_close`]); `HANDLE_SBR_ENCODER` is `&mut SbrEncoder` (or `Option<..>` where C
//!   checks for `NULL`).
//! - The time buffer (`INT_PCM *inputBuffer`, `timeBuffer`, `samples`) is the whole input buffer
//!   of the AAC encoder (`hAacEnc->inputBuffer`), deinterleaved with the channel stride
//!   `inputBufferSizePerChannel`, as the aacenc_lib.cpp calls pass it; the SBR encoder reads its
//!   input from it and writes the downsampled (or PS downmixed) core input into it, at
//!   `downsampledOffset / nChannels` (returned through `inputBufferOffset` of
//!   [`sbr_encoder_init`] as `max(sbrPathOffset, corePathOffset)`).
//! - `sbrData` is `[[u8; MAX_PAYLOAD_SIZE]; 8]` and `sbrDataBits` `[u32; 8]`, one per SBR
//!   element (`UCHAR sbrData[(8)][MAX_PAYLOAD_SIZE]`).
//! - [`sbr_encoder_init`] takes `elInfo` (from `aacEncDistributeSbrBits()`), the
//!   `transformFactor` (1 for ELD, `TRANS_FAC` otherwise), `headerPeriod`, `syntaxFlags` (`AC_LD_MPS`
//!   selects the ELDv2 delay), and `statesInitFlag` (`AACENC_INIT_STATES`); `downSampleFactor` is
//!   the SBR ratio (1: downsampled SBR, 2: dual rate). For `AOT_PS`, it changes `elInfo[0]` to a
//!   mono SCE and `numChannels` to 1, as in C.
//! - The SBR encoder owns its dynamic RAM (`SbrEncoder::dynamic_ram`); the AAC encoder does not
//!   share it (C allocates `Ram_SbrDynamic_RAM` in `sbrEncoder_Open()` as well).
//! - [`sbr_encoder_get_header`] writes `sbr_header()` into an `FdkBitStream` of the caller
//!   (the ASC writer).
//! - `sbrEncoder_GetLibInfo()` takes the `LIB_INFO` array as `&mut [LibInfo]`; the build date and
//!   time are empty (deviation, `SUPPRESS_BUILD_DATE_INFO`).
//! - For non-ELD AOTs the port uses 0 for `sbrConfig[el].init_amp_res_FF`, where the reference
//!   has no defined value; it never reaches the bitstream of a non-ELD stream.

use fdk_aac_rs_fdk::common_fix::FixpDbl;

mod api;
pub use api::*;

/// `MAX_TRANS_FAC`
pub const MAX_TRANS_FAC: i32 = 8;
/// `MAX_CODEC_FRAME_RATIO`
pub const MAX_CODEC_FRAME_RATIO: i32 = 2;
/// `MAX_PAYLOAD_SIZE`
pub const MAX_PAYLOAD_SIZE: usize = 256;

/// `CODEC_TYPE`
pub type CodecType = i32;
/// `CODEC_AAC`
pub const CODEC_AAC: CodecType = 0;
/// `CODEC_AACLD`
pub const CODEC_AACLD: CodecType = 1;
/// `CODEC_UNSPECIFIED`
pub const CODEC_UNSPECIFIED: CodecType = 99;

/// `CODEC_PARAM`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CodecParam {
    /// `bitRate`
    pub bit_rate: i32,
    /// `nChannels`
    pub n_channels: i32,
    /// `sampleFreq`
    pub sample_freq: i32,
    /// `transFac`
    pub trans_fac: i32,
    /// `standardBitrate`
    pub standard_bitrate: i32,
}

/// `SBR_STEREO_MODE`
pub type SbrStereoMode = i32;
/// `SBR_MONO`
pub const SBR_MONO: SbrStereoMode = 0;
/// `SBR_LEFT_RIGHT`
pub const SBR_LEFT_RIGHT: SbrStereoMode = 1;
/// `SBR_COUPLING`
pub const SBR_COUPLING: SbrStereoMode = 2;
/// `SBR_SWITCH_LRC`
pub const SBR_SWITCH_LRC: SbrStereoMode = 3;

/// `SBR_SYNTAX_LOW_DELAY`
pub const SBR_SYNTAX_LOW_DELAY: u32 = 0x0001;
/// `SBR_SYNTAX_SCALABLE`
pub const SBR_SYNTAX_SCALABLE: u32 = 0x0002;
/// `SBR_SYNTAX_CRC`
pub const SBR_SYNTAX_CRC: u32 = 0x0004;
/// `SBR_SYNTAX_DRM_CRC`
pub const SBR_SYNTAX_DRM_CRC: u32 = 0x0008;
/// `SBR_SYNTAX_ELD_REDUCED_DELAY`
pub const SBR_SYNTAX_ELD_REDUCED_DELAY: u32 = 0x0010;

/// `FREQ_RES`
pub type FreqRes = i32;
/// `FREQ_RES_LOW`
pub const FREQ_RES_LOW: FreqRes = 0;
/// `FREQ_RES_HIGH`
pub const FREQ_RES_HIGH: FreqRes = 1;

/// `sbrTuningTable_t`
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SbrTuningTable {
    /// `coreCoder`: LC or ELD
    pub core_coder: CodecType,
    /// `bitrateFrom`: inclusive
    pub bitrate_from: u32,
    /// `bitrateTo`: exclusive
    pub bitrate_to: u32,
    /// `sampleRate`
    pub sample_rate: u32,
    /// `numChannels`
    pub num_channels: u8,
    /// `startFreq`: bs_start_freq
    pub start_freq: u8,
    /// `startFreqSpeech`: bs_start_freq for speech config flag
    pub start_freq_speech: u8,
    /// `stopFreq`: bs_stop_freq
    pub stop_freq: u8,
    /// `stopFreqSpeech`: bs_stop_freq for speech config flag
    pub stop_freq_speech: u8,
    /// `numNoiseBands`
    pub num_noise_bands: u8,
    /// `noiseFloorOffset`
    pub noise_floor_offset: u8,
    /// `noiseMaxLevel`
    pub noise_max_level: i8,
    /// `stereoMode`
    pub stereo_mode: SbrStereoMode,
    /// `freqScale`
    pub freq_scale: u8,
}

/// `sbrConfiguration` (`sbrConfigurationPtr` is `&mut SbrConfiguration`)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SbrConfiguration {
    // core coder dependent configurations
    /// `codecSettings`: Core coder settings. To be set from core coder.
    pub codec_settings: CodecParam,
    /// `SendHeaderDataTime`: SBR header send update frequency in ms.
    pub send_header_data_time: i32,
    /// `useWaveCoding`: Flag: usage of wavecoding tool.
    pub use_wave_coding: i32,
    /// `crcSbr`: Flag: usage of SBR-CRC.
    pub crc_sbr: i32,
    /// `dynBwSupported`: Flag: support for dynamic bandwidth in this combination.
    pub dyn_bw_supported: i32,
    /// `parametricCoding`: Flag: usage of parametric coding tool.
    pub parametric_coding: i32,
    /// `downSampleFactor`: Sampling rate relation between the SBR and the core encoder.
    pub down_sample_factor: i32,
    /// `freq_res_fixfix`: Frequency resolution of envelopes in frame class FIXFIX, for
    /// non-split case and split case
    pub freq_res_fixfix: [FreqRes; 2],
    /// `fResTransIsLow`: Frequency resolution of envelopes in transient frames: low (0) or
    /// variable (1)
    pub f_res_trans_is_low: u8,

    // core coder dependent tuning parameters
    /// `tran_thr`: SBR transient detector threshold (* 100).
    pub tran_thr: i32,
    /// `noiseFloorOffset`: Noise floor offset.
    pub noise_floor_offset: i32,
    /// `useSpeechConfig`: Flag: adapt tuning parameters according to speech.
    pub use_speech_config: u32,

    // core coder independent configurations
    /// `sbrFrameSize`: SBR frame size in samples. Will be calculated from core coder settings.
    pub sbr_frame_size: i32,
    /// `sbr_data_extra`: Flag usage of data extra.
    pub sbr_data_extra: i32,
    /// `amp_res`: Amplitude resolution.
    pub amp_res: i32,
    /// `ana_max_level`: Noise insertion maximum level.
    pub ana_max_level: i32,
    /// `tran_fc`: Transient detector start frequency.
    pub tran_fc: i32,
    /// `tran_det_mode`: Transient detector mode.
    pub tran_det_mode: i32,
    /// `spread`: Flag: usage of SBR spread.
    pub spread: i32,
    /// `stat`: Flag: usage of static framing.
    pub stat: i32,
    /// `e`: Number of envelopes when static framing is chosen.
    pub e: i32,
    /// `stereoMode`: SBR stereo mode.
    pub stereo_mode: SbrStereoMode,
    /// `deltaTAcrossFrames`: Flag: allow time-delta coding.
    pub delta_t_across_frames: i32,
    /// `dF_edge_1stEnv`: Extra fraction delta-F coding is allowed to be more expensive.
    pub d_f_edge_1st_env: FixpDbl,
    /// `dF_edge_incr`: Increment dF_edge_1stEnv this much if dT-coding was used this frame.
    pub d_f_edge_incr: FixpDbl,
    /// `sbr_invf_mode`: Inverse filtering mode.
    pub sbr_invf_mode: i32,
    /// `sbr_xpos_mode`: Transposer mode.
    pub sbr_xpos_mode: i32,
    /// `sbr_xpos_ctrl`: Transposer control.
    pub sbr_xpos_ctrl: i32,
    /// `sbr_xpos_level`: Transposer 3rd order level.
    pub sbr_xpos_level: i32,
    /// `startFreq`: The start frequency table index.
    pub start_freq: i32,
    /// `stopFreq`: The stop frequency table index.
    pub stop_freq: i32,
    /// `useSaPan`: Flag: usage of SAPAN stereo.
    pub use_sa_pan: i32,
    /// `dynBwEnabled`: Flag: usage of dynamic bandwidth.
    pub dyn_bw_enabled: i32,
    /// `bParametricStereo`: Flag: usage of parametric stereo coding tool.
    pub b_parametric_stereo: i32,

    // header_extra1 configuration
    /// `freqScale`: Frequency grouping.
    pub freq_scale: u8,
    /// `alterScale`: Scale resolution.
    pub alter_scale: i32,
    /// `sbr_noise_bands`: Number of noise bands.
    pub sbr_noise_bands: i32,

    // header_extra2 configuration
    /// `sbr_limiter_bands`: Number of limiter bands.
    pub sbr_limiter_bands: i32,
    /// `sbr_limiter_gains`: Gain of limiter.
    pub sbr_limiter_gains: i32,
    /// `sbr_interpol_freq`: Flag: use interpolation in freq. direction.
    pub sbr_interpol_freq: i32,
    /// `sbr_smoothing_length`: Flag: choose length 4 or 0 (=on, off).
    pub sbr_smoothing_length: i32,
    /// `init_amp_res_FF`
    pub init_amp_res_ff: u8,
    /// `threshold_AmpRes_FF_m`
    pub threshold_amp_res_ff_m: FixpDbl,
    /// `threshold_AmpRes_FF_e`
    pub threshold_amp_res_ff_e: i8,
}

/// `SBR_CONFIG_DATA` (`HANDLE_SBR_CONFIG_DATA` is `&mut SbrConfigData`).
///
/// The frequency band tables and the master table are pointers to per element allocations in
/// C (`Ram_Sbr_freqBandTableLO`, `..HI`, `Ram_Sbr_v_k_master`); they are owned arrays here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SbrConfigData {
    /// `sbrSyntaxFlags`: SBR syntax flags derived from AOT.
    pub sbr_syntax_flags: u32,
    /// `nChannels`: Number of channels.
    pub n_channels: i32,
    /// `nSfb`: Number of SBR scalefactor bands for LO_RES and HI_RES
    pub n_sfb: [i32; 2],
    /// `num_Master`: Number of elements in v_k_master.
    pub num_master: i32,
    /// `sampleFreq`: SBR sampling frequency.
    pub sample_freq: i32,
    /// `frameSize`
    pub frame_size: i32,
    /// `xOverFreq`: The SBR start frequency.
    pub x_over_freq: i32,
    /// `dynXOverFreq`: Used crossover frequency when dynamic bandwidth is enabled.
    pub dyn_x_over_freq: i32,
    /// `noQmfBands`: Number of QMF frequency bands.
    pub no_qmf_bands: i32,
    /// `noQmfSlots`: Number of QMF slots.
    pub no_qmf_slots: i32,
    /// `freqBandTable[LO]`: low resolution frequency table (`MAX_FREQ_COEFFS / 2 + 1`)
    pub freq_band_table_lo: [u8; crate::sbr_def::MAX_FREQ_COEFFS / 2 + 1],
    /// `freqBandTable[HI]`: high resolution frequency table (`MAX_FREQ_COEFFS + 1`)
    pub freq_band_table_hi: [u8; crate::sbr_def::MAX_FREQ_COEFFS + 1],
    /// `v_k_master`: Master BandTable where freqBandTable is derived from.
    pub v_k_master: [u8; crate::sbr_def::MAX_FREQ_COEFFS + 1],
    /// `stereoMode`
    pub stereo_mode: SbrStereoMode,
    /// `noEnvChannels`: Number of envelope channels.
    pub no_env_channels: i32,
    /// `useWaveCoding`: Flag indicates whether to use wave coding at all.
    pub use_wave_coding: i32,
    /// `useParametricCoding`: Flag indicates whether to use para coding at all.
    pub use_parametric_coding: i32,
    /// `xposCtrlSwitch`: Flag indicates whether to switch xpos ctrl on the fly.
    pub xpos_ctrl_switch: i32,
    /// `switchTransposers`: Flag indicates whether to switch xpos on the fly .
    pub switch_transposers: i32,
    /// `initAmpResFF`
    pub init_amp_res_ff: u8,
    /// `thresholdAmpResFF_m`
    pub threshold_amp_res_ff_m: FixpDbl,
    /// `thresholdAmpResFF_e`
    pub threshold_amp_res_ff_e: i8,
}

impl Default for SbrConfigData {
    fn default() -> Self {
        SbrConfigData {
            sbr_syntax_flags: 0,
            n_channels: 0,
            n_sfb: [0; 2],
            num_master: 0,
            sample_freq: 0,
            frame_size: 0,
            x_over_freq: 0,
            dyn_x_over_freq: 0,
            no_qmf_bands: 0,
            no_qmf_slots: 0,
            freq_band_table_lo: [0; crate::sbr_def::MAX_FREQ_COEFFS / 2 + 1],
            freq_band_table_hi: [0; crate::sbr_def::MAX_FREQ_COEFFS + 1],
            v_k_master: [0; crate::sbr_def::MAX_FREQ_COEFFS + 1],
            stereo_mode: 0,
            no_env_channels: 0,
            use_wave_coding: 0,
            use_parametric_coding: 0,
            xpos_ctrl_switch: 0,
            switch_transposers: 0,
            init_amp_res_ff: 0,
            threshold_amp_res_ff_m: 0,
            threshold_amp_res_ff_e: 0,
        }
    }
}

impl SbrConfigData {
    /// `freqBandTable[res]`
    pub fn freq_band_table(&self, res: usize) -> &[u8] {
        if res == crate::sbr_def::LO {
            &self.freq_band_table_lo
        } else {
            &self.freq_band_table_hi
        }
    }
}

/// `SBR_ELEMENT_INFO`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SbrElementInfo {
    /// `elType`
    pub el_type: fdk_aac_rs_fdk::audio::Mp4ElementId,
    /// `bitRate`
    pub bit_rate: i32,
    /// `instanceTag`
    pub instance_tag: i32,
    /// `fParametricStereo`
    pub f_parametric_stereo: u8,
    /// `fDualMono`: This flags allows to disable coupling in sbr channel pair element
    pub f_dual_mono: u8,
    /// `nChannelsInEl`
    pub n_channels_in_el: u8,
    /// `ChannelIndex`
    pub channel_index: [u8; 2],
}
