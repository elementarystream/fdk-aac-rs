// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libSBRdec/src/env_extr.h, libSBRdec/src/env_extr.cpp.

//! Port of `env_extr.h` and `env_extr.cpp`: the SBR header and frame data types and the
//! bitstream parser of the SBR header (`sbrGetHeaderData`) and of the SBR channel elements
//! (`sbrGetChannelElement`), including the USAC, ELD (low delay grid) and PVC variants.
//!
//! The parser is the first consumer of untrusted data: every index derived from the bitstream
//! is either limited by the C code itself or checked here (see the notes at the functions).

use crate::huff_dec::{decode_huffman_cw, Huffman};
use crate::lpp_tran::*;
use crate::psbitdec::read_ps_data;
use crate::psdec::PsDec;
use crate::pvc_dec::{MAX_PVC_ENVELOPES, PVC_DIVMODE_BITS, PVC_NTIMESLOT, PVC_PVCID_BITS};
use crate::sbr_ram::{pod_bytes, pod_struct};
use crate::sbr_rom::*;
use crate::sbrdecoder::{SbrError, SBRDEC_OK, SBRDEC_UNSUPPORTED_CONFIG};
use fdk_aac_rs_fdk::audio::AC_CM_DET_CFG_CHANGE;
use fdk_aac_rs_fdk::bitstream::*;
use fdk_aac_rs_fdk::common_fix::*;
use fdk_aac_rs_fdk::fdk_assert;

pub mod tables;
use tables::*;

/// `ENV_EXP_FRACT`: shift of the raw envelope data to support fractional numbers (0: none).
pub const ENV_EXP_FRACT: i32 = 0;
/// `EXP_BITS`: size of the exponent part of a pseudo float envelope value.
pub const EXP_BITS: i32 = 6;
/// `MASK_M`: mask for extracting the mantissa of a pseudo float envelope value
pub const MASK_M: i32 = ((1 << (FRACT_BITS - EXP_BITS)) - 1) << EXP_BITS;
/// `MASK_E`: mask for extracting the exponent of a pseudo float envelope value
pub const MASK_E: i32 = (1 << EXP_BITS) - 1;
/// `SIGN_EXT`: a CHAR-constant with all bits above our sign-bit set
pub const SIGN_EXT: i32 = (-1i8 as i32) ^ MASK_E;
/// `ROUNDING`: 0.5-offset for rounding the mantissa of a pseudo-float envelope value
pub const ROUNDING: FixpSgl = 1 << (EXP_BITS - 1);
/// `NRG_EXP_OFFSET`: added to the reference energy's exponent to prevent negative numbers
pub const NRG_EXP_OFFSET: i32 = 16;
/// `NOISE_EXP_OFFSET`: added to the noise level exponent to prevent negative numbers
pub const NOISE_EXP_OFFSET: i32 = 38;

/// `ADD_HARMONICS_FLAGS_SIZE`: `ceil(MAX_FREQ_COEFFS/32)`
pub const ADD_HARMONICS_FLAGS_SIZE: usize = 2;

/// `SBR_HEADER_STATUS`
pub type SbrHeaderStatus = i32;
/// `HEADER_NOT_PRESENT`
pub const HEADER_NOT_PRESENT: SbrHeaderStatus = 0;
/// `HEADER_ERROR`
pub const HEADER_ERROR: SbrHeaderStatus = 1;
/// `HEADER_OK`
pub const HEADER_OK: SbrHeaderStatus = 2;
/// `HEADER_RESET`
pub const HEADER_RESET: SbrHeaderStatus = 3;

/// `SBR_SYNC_STATE`
pub type SbrSyncState = i32;
/// `SBR_NOT_INITIALIZED`
pub const SBR_NOT_INITIALIZED: SbrSyncState = 0;
/// `UPSAMPLING`
pub const UPSAMPLING: SbrSyncState = 1;
/// `SBR_HEADER`
pub const SBR_HEADER: SbrSyncState = 2;
/// `SBR_ACTIVE`
pub const SBR_ACTIVE: SbrSyncState = 3;

/// `COUPLING_MODE`
pub type CouplingMode = i32;
/// `COUPLING_OFF`
pub const COUPLING_OFF: CouplingMode = 0;
/// `COUPLING_LEVEL`
pub const COUPLING_LEVEL: CouplingMode = 1;
/// `COUPLING_BAL`
pub const COUPLING_BAL: CouplingMode = 2;

pod_struct! {
    /// `FREQ_BAND_DATA`
    ///
    /// `freqBandTable[2]` always points to `freqBandTableLo` and `freqBandTableHi` in the
    /// reference (`initHeaderData()`, `copySbrHeader()`), apart from a temporary redirection in
    /// `calculateSbrEnvelope()`, which the port keeps local (see [`crate::env_calc`]); the
    /// pointers are a placeholder here.
    pub struct FreqBandData {
        /// `nSfb`: number of SBR-bands for low and high freq-resolution
        pub n_sfb: [u8; 2],
        /// `nNfb`: actual number of noise bands to read from the bitstream
        pub n_nfb: u8,
        /// `numMaster`: number of SBR-bands in v_k_master
        pub num_master: u8,
        /// `lowSubband`: QMF-band where SBR frequency range starts
        pub low_subband: u8,
        /// `highSubband`: QMF-band where SBR frequency range ends
        pub high_subband: u8,
        /// `ov_highSubband`: the old highband value if a header change applies (overlap area)
        pub ov_high_subband: u8,
        /// `limiterBandTable`: limiter band table
        pub limiter_band_table: [u8; MAX_NUM_LIMITERS + 1],
        /// `noLimiterBands`: number of limiter bands
        pub no_limiter_bands: u8,
        /// `nInvfBands`: number of bands for inverse filtering
        pub n_invf_bands: u8,
        pub _pad0: [u8; 2],
        /// `freqBandTable[2]` (placeholder, see above)
        pub _freq_band_table: [usize; 2],
        /// `freqBandTableLo`: mapping of SBR bands to QMF bands for low frequency resolution
        pub freq_band_table_lo: [u8; MAX_FREQ_COEFFS / 2 + 1],
        /// `freqBandTableHi`: mapping of SBR bands to QMF bands for high frequency resolution
        pub freq_band_table_hi: [u8; MAX_FREQ_COEFFS + 1],
        /// `freqBandTableNoise`: mapping of SBR noise bands to QMF bands
        pub freq_band_table_noise: [u8; MAX_NOISE_COEFFS + 1],
        /// `v_k_master`: master band table which freqBandTable is derived from
        pub v_k_master: [u8; MAX_FREQ_COEFFS + 1],
        pub _pad1: [u8; 3],
    }
}

impl FreqBandData {
    /// `freqBandTable[i]` (0: low resolution, 1: high resolution).
    #[inline]
    pub fn freq_band_table(&self, i: usize) -> &[u8] {
        if i == 0 {
            &self.freq_band_table_lo
        } else {
            &self.freq_band_table_hi
        }
    }

    /// `nSfb[i]`: a value of `freqRes` outside 0..=1 indexes the following bytes of the
    /// structure in C (`nNfb`, `numMaster`, ...), which the port reproduces.
    #[inline]
    pub fn n_sfb_at(&self, i: usize) -> u8 {
        pod_bytes(self).get(i).copied().unwrap_or(0)
    }
}

/// `SBRDEC_ELD_GRID`
pub const SBRDEC_ELD_GRID: u32 = 1;
/// `SBRDEC_SYNTAX_SCAL`
pub const SBRDEC_SYNTAX_SCAL: u32 = 2;
/// `SBRDEC_SYNTAX_USAC`
pub const SBRDEC_SYNTAX_USAC: u32 = 4;
/// `SBRDEC_SYNTAX_RSVD50`
pub const SBRDEC_SYNTAX_RSVD50: u32 = 8;
/// `SBRDEC_USAC_INDEP`: USAC global independency flag is active.
pub const SBRDEC_USAC_INDEP: u32 = 16;
/// `SBRDEC_LOW_POWER`: Low Power QMF mode shall be used.
pub const SBRDEC_LOW_POWER: u32 = 32;
/// `SBRDEC_PS_DECODED`: PS was decoded and rendered.
pub const SBRDEC_PS_DECODED: u32 = 64;
/// `SBRDEC_QUAD_RATE`: USAC SBR 4:1 is active.
pub const SBRDEC_QUAD_RATE: u32 = 128;
/// `SBRDEC_USAC_HARMONICSBR`: USAC HBE tool is active.
pub const SBRDEC_USAC_HARMONICSBR: u32 = 256;
/// `SBRDEC_LD_MPS_QMF`: the LD-MPS QMF shall be used.
pub const SBRDEC_LD_MPS_QMF: u32 = 512;
/// `SBRDEC_USAC_ITES`: USAC inter TES tool is active.
pub const SBRDEC_USAC_ITES: u32 = 1024;
/// `SBRDEC_SYNTAX_DRM`: DRM30/DRM+ reverse syntax is being used.
pub const SBRDEC_SYNTAX_DRM: u32 = 2048;
/// `SBRDEC_ELD_DOWNSCALE`: ELD downscaled mode decoding is used
pub const SBRDEC_ELD_DOWNSCALE: u32 = 4096;
/// `SBRDEC_DOWNSAMPLE`: the downsampling mode is used.
pub const SBRDEC_DOWNSAMPLE: u32 = 8192;
/// `SBRDEC_FLUSH`: flush all elements in use.
pub const SBRDEC_FLUSH: u32 = 16384;
/// `SBRDEC_FORCE_RESET`: force a reset of all elements in use.
pub const SBRDEC_FORCE_RESET: u32 = 32768;
/// `SBRDEC_SKIP_QMF_ANA`: the input data is provided in the QMF domain.
pub const SBRDEC_SKIP_QMF_ANA: u32 = 1 << 21;
/// `SBRDEC_SKIP_QMF_SYN`: the output data is exported in the QMF domain.
pub const SBRDEC_SKIP_QMF_SYN: u32 = 1 << 22;

/// `SBRDEC_HDR_STAT_RESET`
pub const SBRDEC_HDR_STAT_RESET: u8 = 1;
/// `SBRDEC_HDR_STAT_UPDATE`
pub const SBRDEC_HDR_STAT_UPDATE: u8 = 2;

pod_struct! {
    /// `SBR_HEADER_DATA_BS_INFO`
    pub struct SbrHeaderDataBsInfo {
        /// `ampResolution`: amplitude resolution of envelope values (0: 1.5dB, 1: 3dB)
        pub amp_resolution: u8,
        /// `xover_band`: start index in `v_k_master[]` used for dynamic crossover frequency
        pub xover_band: u8,
        /// `sbr_preprocessing`: SBR prewhitening flag
        pub sbr_preprocessing: u8,
        /// `pvc_mode`: predictive vector coding mode
        pub pvc_mode: u8,
    }
}

pod_struct! {
    /// `SBR_HEADER_DATA_BS`
    pub struct SbrHeaderDataBs {
        /// `startFreq`: index for SBR start frequency
        pub start_freq: u8,
        /// `stopFreq`: index for SBR highest frequency
        pub stop_freq: u8,
        /// `freqScale`: 0: linear scale, 1-3 logarithmic scales
        pub freq_scale: u8,
        /// `alterScale`: flag for coarser frequency resolution
        pub alter_scale: u8,
        /// `noise_bands`: noise bands per octave, read from bitstream
        pub noise_bands: u8,
        /// `limiterBands`: index for number of limiter bands per octave
        pub limiter_bands: u8,
        /// `limiterGains`: index to select gain limit
        pub limiter_gains: u8,
        /// `interpolFreq`: gain calculation method (1: per QMF channel, 0: per SBR band)
        pub interpol_freq: u8,
        /// `smoothingLength`: smoothing of gains over time (0: on  1: off)
        pub smoothing_length: u8,
    }
}

pod_struct! {
    /// `SBR_HEADER_DATA`
    pub struct SbrHeaderData {
        /// `syncState`: the current initialization status of the header
        pub sync_state: SbrSyncState,
        /// `status`: reset/update flags (`SBRDEC_HDR_STAT_*`)
        pub status: u8,
        /// `frameErrorFlag`: frame data valid flag
        pub frame_error_flag: u8,
        /// `numberTimeSlots`: AAC: 16,15
        pub number_time_slots: u8,
        /// `numberOfAnalysisBands`: number of QMF analysis bands
        pub number_of_analysis_bands: u8,
        /// `timeStep`: time resolution of SBR in QMF-slots
        pub time_step: u8,
        pub _pad0: [u8; 3],
        /// `sbrProcSmplRate`: SBR processing sampling frequency
        pub sbr_proc_smpl_rate: u32,
        /// `bs_data`: current SBR header
        pub bs_data: SbrHeaderDataBs,
        /// `bs_dflt`: default sbr header
        pub bs_dflt: SbrHeaderDataBs,
        /// `bs_info`: SBR info
        pub bs_info: SbrHeaderDataBsInfo,
        pub _pad1: [u8; 2],
        /// `freqBandData`
        pub freq_band_data: FreqBandData,
        /// `pvcIDprev`
        pub pvc_id_prev: u8,
        /// Up to the alignment of the structure (8 with 8-byte pointers, 4 with 4-byte ones).
        pub _pad2: [u8; if cfg!(target_pointer_width = "32") { 3 } else { 7 }],
    }
}

pod_struct! {
    /// `FRAME_INFO`
    pub struct FrameInfo {
        /// `frameClass`: select grid type
        pub frame_class: u8,
        /// `nEnvelopes`: number of envelopes
        pub n_envelopes: u8,
        /// `borders`: envelope borders (in SBR-timeslots)
        pub borders: [u8; MAX_ENVELOPES + 1],
        /// `freqRes`: frequency resolution for each envelope (0=low, 1=high)
        pub freq_res: [u8; MAX_ENVELOPES],
        /// `tranEnv`: transient envelope, -1 if none
        pub tran_env: i8,
        /// `nNoiseEnvelopes`: number of noise envelopes
        pub n_noise_envelopes: u8,
        /// `bordersNoise`: borders of noise envelopes
        pub borders_noise: [u8; MAX_NOISE_ENVELOPES + 1],
        /// `pvcBorders`
        pub pvc_borders: [u8; MAX_PVC_ENVELOPES + 1],
        /// `noisePosition`
        pub noise_position: u8,
        /// `varLength`
        pub var_length: u8,
    }
}

pod_struct! {
    /// `SBR_PREV_FRAME_DATA`
    pub struct SbrPrevFrameData {
        /// `sfb_nrg_prev`: previous envelope (required for differential-coded values)
        pub sfb_nrg_prev: [FixpSgl; MAX_FREQ_COEFFS],
        /// `prevNoiseLevel`: previous noise envelope (required for differential-coded values)
        pub prev_noise_level: [FixpSgl; MAX_NOISE_COEFFS],
        pub _pad0: [u8; 2],
        /// `coupling`: stereo-mode of previous frame
        pub coupling: CouplingMode,
        /// `sbr_invf_mode`: previous strength of filtering in transposer
        pub sbr_invf_mode: [InvfMode; MAX_INVF_BANDS],
        /// `ampRes`: previous amplitude resolution (0: 1.5dB, 1: 3dB)
        pub amp_res: u8,
        /// `stopPos`: position in time where last envelope ended
        pub stop_pos: u8,
        /// `frameErrorFlag`: previous frame status
        pub frame_error_flag: u8,
        /// `prevSbrPitchInBins`: previous frame pitchInBins
        pub prev_sbr_pitch_in_bins: u8,
        /// `prevFrameInfo`
        pub prev_frame_info: FrameInfo,
        pub _pad1: [u8; 3],
    }
}

pod_struct! {
    /// `SBR_FRAME_DATA`
    pub struct SbrFrameData {
        /// `nScaleFactors`: total number of scalefactors in frame
        pub n_scale_factors: i32,
        /// `frameInfo`: time grid for current frame
        pub frame_info: FrameInfo,
        /// `domain_vec`: direction of delta-coding for each envelope (0:frequency, 1:time)
        pub domain_vec: [u8; MAX_ENVELOPES],
        /// `domain_vec_noise`: same as above, but for noise envelopes
        pub domain_vec_noise: [u8; MAX_NOISE_ENVELOPES],
        pub _pad0: [u8; 1],
        /// `sbr_invf_mode`: strength of filtering in transposer
        pub sbr_invf_mode: [InvfMode; MAX_INVF_BANDS],
        /// `coupling`: stereo-mode
        pub coupling: CouplingMode,
        /// `ampResolutionCurrentFrame`: amplitude resolution of envelope values
        pub amp_resolution_current_frame: i32,
        /// `addHarmonics`: flags for synthetic sine addition (aligned to MSB)
        pub add_harmonics: [u32; ADD_HARMONICS_FLAGS_SIZE],
        /// `iEnvelope`: envelope data
        pub i_envelope: [FixpSgl; MAX_NUM_ENVELOPE_VALUES],
        /// `sbrNoiseFloorLevel`: noise envelope data
        pub sbr_noise_floor_level: [FixpSgl; MAX_NUM_NOISE_VALUES],
        /// `iTESactive`: one flag for each envelope to enable USAC inter-TES
        pub i_tes_active: u8,
        /// `interTempShapeMode`: USAC inter-TES: bs_inter_temp_shape_mode[ch][env] value
        pub inter_temp_shape_mode: [u8; MAX_ENVELOPES],
        /// `pvcID`: one PVC ID value for each time slot
        pub pvc_id: [u8; PVC_NTIMESLOT],
        /// `ns`
        pub ns: u8,
        /// `sinusoidal_position`
        pub sinusoidal_position: u8,
        /// `sbrPatchingMode`
        pub sbr_patching_mode: u8,
        /// `sbrOversamplingFlag`
        pub sbr_oversampling_flag: u8,
        /// `sbrPitchInBins`
        pub sbr_pitch_in_bins: u8,
        pub _pad1: [u8; 2],
    }
}

/// `SR_MAPPING`: if `fsRangeLo(n+1) > fs >= fsRangeLo(n)`, fs is mapped to `fsMapped`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SrMapping {
    /// `fsRangeLo`
    pub fs_range_lo: u32,
    /// `fsMapped`
    pub fs_mapped: u32,
}

/// `DRM_PARAMETRIC_STEREO`
const _DRM_PARAMETRIC_STEREO: i32 = 0;
/// `EXTENSION_ID_PS_CODING`
const EXTENSION_ID_PS_CODING: i32 = 2;

/// `sbrdec_mapToStdSampleRate(fs, isUsac)`: maps an arbitrary sampling frequency to the nearest
/// frequency for which setup tables are available (14496-3, 4.6.18.2.6).
pub fn sbrdec_map_to_std_sample_rate(fs: u32, is_usac: u32) -> u32 {
    let mapping_table: &[SrMapping] =
        if is_usac == 0 { &STD_SAMPLE_RATES_MAPPING } else { &STD_SAMPLE_RATES_MAPPING_USAC };

    for m in mapping_table.iter().rev() {
        if fs >= m.fs_range_lo {
            return m.fs_mapped;
        }
    }
    fs
}

/// `initHeaderData()`: copies default values to the header data and patches some entries
/// depending on the core codec.
pub fn init_header_data(
    h_header_data: &mut SbrHeaderData,
    sample_rate_in: i32,
    sample_rate_out: i32,
    downscale_factor: i32,
    samples_per_frame: i32,
    flags: u32,
    set_default_hdr: i32,
) -> SbrError {
    let mut sbr_error = SBRDEC_OK;
    let mut num_analysis_bands: i32;

    // The products of the sampling rates and the downscale factor cannot overflow for the rates
    // of the library (at most 96000 * 4).
    let rate_ds = sample_rate_out.wrapping_mul(downscale_factor);
    let sample_rate_proc = if flags & (SBRDEC_SYNTAX_USAC | SBRDEC_SYNTAX_RSVD50) == 0 {
        sbrdec_map_to_std_sample_rate(rate_ds as u32, 0) as i32
    } else {
        rate_ds
    };

    if sample_rate_in == sample_rate_out {
        h_header_data.sbr_proc_smpl_rate = (sample_rate_proc << 1) as u32;
        num_analysis_bands = 32;
    } else {
        h_header_data.sbr_proc_smpl_rate = sample_rate_proc as u32;
        if (sample_rate_out >> 1) == sample_rate_in {
            // 1:2
            num_analysis_bands = 32;
        } else if (sample_rate_out >> 2) == sample_rate_in {
            // 1:4
            num_analysis_bands = 16;
        } else if (sample_rate_out.wrapping_mul(3) >> 3) == (sample_rate_in.wrapping_mul(8) >> 3) {
            // 3:8, 3/4 core frame length
            num_analysis_bands = 24;
        } else {
            return SBRDEC_UNSUPPORTED_CONFIG;
        }
    }
    // Guarded: a downscale factor of 0 gives 0 bands.
    num_analysis_bands =
        if downscale_factor != 0 { num_analysis_bands / downscale_factor } else { 0 };

    if set_default_hdr != 0 {
        // Fill in default values first
        h_header_data.sync_state = SBR_NOT_INITIALIZED;
        h_header_data.status = 0;
        h_header_data.frame_error_flag = 0;

        h_header_data.bs_info.amp_resolution = 1;
        h_header_data.bs_info.xover_band = 0;
        h_header_data.bs_info.sbr_preprocessing = 0;
        h_header_data.bs_info.pvc_mode = 0;

        h_header_data.bs_data.start_freq = 5;
        h_header_data.bs_data.stop_freq = 0;
        // previously 2; for ELD reduced delay bitstreams /samplerates initializing of the sbr
        // decoder instance fails if freqScale is set to 2 because no master table can be
        // generated; in ELD reduced delay bitstreams this value is always 0; gets overwritten
        // when header is read
        h_header_data.bs_data.freq_scale = 0;
        h_header_data.bs_data.alter_scale = 1;
        h_header_data.bs_data.noise_bands = 2;
        h_header_data.bs_data.limiter_bands = 2;
        h_header_data.bs_data.limiter_gains = 2;
        h_header_data.bs_data.interpol_freq = 1;
        h_header_data.bs_data.smoothing_length = 1;

        // Patch some entries
        if rate_ds >= 96000 {
            // having read these frequency values from bit stream before.
            h_header_data.bs_data.start_freq = 4;
            h_header_data.bs_data.stop_freq = 3;
        } else if rate_ds > 24000 {
            // Trigger an error if SBR is going to be processed without having read these
            // frequency values from bit stream before.
            h_header_data.bs_data.start_freq = 7;
            h_header_data.bs_data.stop_freq = 3;
        }
    }

    if (sample_rate_out >> 2) == sample_rate_in {
        h_header_data.time_step = 4;
    } else {
        h_header_data.time_step = if flags & SBRDEC_ELD_GRID != 0 { 1 } else { 2 };
    }

    // Setup pointers to frequency band tables (freqBandTable[0] = freqBandTableLo,
    // freqBandTable[1] = freqBandTableHi): implicit in the port.

    // One SBR timeslot corresponds to the amount of samples equal to the amount of analysis
    // bands, divided by the timestep. (UCHAR = (int >> (timeStep - 1)))
    // Guarded: a band count of 0 gives 0 slots.
    let slots = if num_analysis_bands != 0 { samples_per_frame / num_analysis_bands } else { 0 };
    h_header_data.number_time_slots = (slots >> (h_header_data.time_step as i32 - 1)) as u8;
    if h_header_data.number_time_slots > 16 {
        sbr_error = SBRDEC_UNSUPPORTED_CONFIG;
    }

    h_header_data.number_of_analysis_bands = num_analysis_bands as u8;
    if (sample_rate_out >> 2) == sample_rate_in {
        h_header_data.number_time_slots <<= 1;
    }

    sbr_error
}

/// `initSbrPrevFrameData(h_prev_data, timeSlots)`: initializes the previous frame data.
pub fn init_sbr_prev_frame_data(h_prev_data: &mut SbrPrevFrameData, time_slots: i32) {
    // Set previous energy and noise levels to 0 for the case that decoding starts in the middle
    // of a bitstream
    h_prev_data.sfb_nrg_prev.fill(0);
    h_prev_data.prev_noise_level.fill(0);
    h_prev_data.sbr_invf_mode.fill(INVF_OFF);

    h_prev_data.stop_pos = time_slots as u8;
    h_prev_data.coupling = COUPLING_OFF;
    h_prev_data.amp_res = 0;

    h_prev_data.prev_frame_info = FrameInfo::ZERO;
}

/// `sbrGetHeaderData(headerData, hBs, flags, fIsSbrData, configMode)`: reads the header data
/// from the bitstream. `h_header_data` is `None` (`NULL`) only in the configuration change
/// detection mode (`configMode & AC_CM_DET_CFG_CHANGE`), where the header is read and discarded.
pub fn sbr_get_header_data(
    h_header_data: Option<&mut SbrHeaderData>,
    h_bs: &mut FdkBitStream,
    flags: u32,
    f_is_sbr_data: i32,
    config_mode: u8,
) -> SbrHeaderStatus {
    // Read and discard new header in config change detection mode
    if config_mode & AC_CM_DET_CFG_CHANGE != 0 {
        if flags & (SBRDEC_SYNTAX_RSVD50 | SBRDEC_SYNTAX_USAC) == 0 {
            // ampResolution
            fdk_read_bits(h_bs, 1);
        }
        // startFreq, stopFreq
        fdk_push_for(h_bs, 8);
        if flags & (SBRDEC_SYNTAX_RSVD50 | SBRDEC_SYNTAX_USAC) == 0 {
            // xover_band
            fdk_read_bits(h_bs, 3);
            // reserved bits
            fdk_read_bits(h_bs, 2);
        }
        let header_extra1 = fdk_read_bit(h_bs);
        let header_extra2 = fdk_read_bit(h_bs);
        fdk_push_for(h_bs, 5 * header_extra1 + 6 * header_extra2);

        return HEADER_OK;
    }

    // The decoder passes a header in every other mode.
    let Some(h) = h_header_data else {
        return HEADER_OK;
    };

    // Copy SBR bit stream header to temporary header
    let last_header = h.bs_data;
    let last_info = h.bs_info;

    // Read new header from bitstream
    let use_dflt = (flags & (SBRDEC_SYNTAX_RSVD50 | SBRDEC_SYNTAX_USAC)) != 0 && f_is_sbr_data == 0;

    if flags & (SBRDEC_SYNTAX_RSVD50 | SBRDEC_SYNTAX_USAC) == 0 {
        h.bs_info.amp_resolution = fdk_read_bits(h_bs, 1) as u8;
    }

    let mut bs = if use_dflt { h.bs_dflt } else { h.bs_data };

    bs.start_freq = fdk_read_bits(h_bs, 4) as u8;
    bs.stop_freq = fdk_read_bits(h_bs, 4) as u8;

    if flags & (SBRDEC_SYNTAX_RSVD50 | SBRDEC_SYNTAX_USAC) == 0 {
        h.bs_info.xover_band = fdk_read_bits(h_bs, 3) as u8;
        fdk_read_bits(h_bs, 2);
    }

    let header_extra1 = fdk_read_bits(h_bs, 1);
    let header_extra2 = fdk_read_bits(h_bs, 1);

    // Handle extra header information
    if header_extra1 != 0 {
        bs.freq_scale = fdk_read_bits(h_bs, 2) as u8;
        bs.alter_scale = fdk_read_bits(h_bs, 1) as u8;
        bs.noise_bands = fdk_read_bits(h_bs, 2) as u8;
    } else {
        bs.freq_scale = 2;
        bs.alter_scale = 1;
        bs.noise_bands = 2;
    }

    if header_extra2 != 0 {
        bs.limiter_bands = fdk_read_bits(h_bs, 2) as u8;
        bs.limiter_gains = fdk_read_bits(h_bs, 2) as u8;
        bs.interpol_freq = fdk_read_bits(h_bs, 1) as u8;
        bs.smoothing_length = fdk_read_bits(h_bs, 1) as u8;
    } else {
        bs.limiter_bands = 2;
        bs.limiter_gains = 2;
        bs.interpol_freq = 1;
        bs.smoothing_length = 1;
    }

    if use_dflt {
        h.bs_dflt = bs;
    } else {
        h.bs_data = bs;
    }

    // Look for new settings. IEC 14496-3, 4.6.18.3.1
    if h.sync_state < SBR_HEADER
        || last_header.start_freq != bs.start_freq
        || last_header.stop_freq != bs.stop_freq
        || last_header.freq_scale != bs.freq_scale
        || last_header.alter_scale != bs.alter_scale
        || last_header.noise_bands != bs.noise_bands
        || last_info.xover_band != h.bs_info.xover_band
    {
        return HEADER_RESET; // New settings
    }

    HEADER_OK
}

/// `sbrGetSyntheticCodedData()`: gets the missing harmonics parameters. Returns the number of
/// bits read (without the sinusoidal position of USAC).
pub fn sbr_get_synthetic_coded_data(
    h_header_data: &SbrHeaderData,
    h_frame_data: &mut SbrFrameData,
    h_bs: &mut FdkBitStream,
    flags: u32,
) -> i32 {
    let mut bits_read = 0;

    let add_harmonic_flag = fdk_read_bits(h_bs, 1);
    bits_read += 1;

    if add_harmonic_flag != 0 {
        let mut n_sfb = h_header_data.freq_band_data.n_sfb[1] as i32;
        for i in 0..ADD_HARMONICS_FLAGS_SIZE {
            // read maximum 32 bits and align them to the MSB
            let read_bits = f_min(32, n_sfb);
            n_sfb -= read_bits;
            if read_bits > 0 {
                // FDKreadBits(hBs, readBits) << (32 - readBits): UINT shift, 0..31
                h_frame_data.add_harmonics[i] =
                    fdk_read_bits(h_bs, read_bits as u32) << (32 - read_bits);
            } else {
                h_frame_data.add_harmonics[i] = 0;
            }

            bits_read += read_bits;
        }
        // bs_pvc_mode = 0 for Rsvd50
        if flags & SBRDEC_SYNTAX_USAC != 0 && h_header_data.bs_info.pvc_mode != 0 {
            let mut bs_sinusoidal_position = 31;
            if fdk_read_bit(h_bs) != 0 {
                // bs_sinusoidal_position_flag
                bs_sinusoidal_position = fdk_read_bits(h_bs, 5);
            }
            h_frame_data.sinusoidal_position = bs_sinusoidal_position as u8;
        }
    } else {
        h_frame_data.add_harmonics = [0; ADD_HARMONICS_FLAGS_SIZE];
    }

    bits_read
}

/// `extractExtendedData()`: reads the extension data elements; only PS data is decoded, the
/// others are skipped. Returns 1 if the frame is ok.
fn extract_extended_data(
    _h_header_data: &SbrHeaderData,
    h_bs: &mut FdkBitStream,
    mut h_parametric_stereo_dec: Option<&mut PsDec>,
) -> i32 {
    let mut frame_ok = 1;

    let extended_data = fdk_read_bits(h_bs, 1);

    if extended_data != 0 {
        let mut b_ps_read = false;

        let mut cnt = fdk_read_bits(h_bs, 4) as i32;
        if cnt == (1 << 4) - 1 {
            cnt += fdk_read_bits(h_bs, 8) as i32;
        }

        let mut n_bits_left = 8 * cnt;

        // sanity check for cnt
        if n_bits_left > fdk_get_valid_bits(h_bs) as i32 {
            // limit nBitsLeft
            n_bits_left = fdk_get_valid_bits(h_bs) as i32;
            // set frame error
            frame_ok = 0;
        }

        while n_bits_left > 7 {
            let extension_id = fdk_read_bits(h_bs, 2) as i32;
            n_bits_left -= 2;

            if extension_id == EXTENSION_ID_PS_CODING {
                // Read PS data from bitstream
                if let Some(ps) = h_parametric_stereo_dec.as_deref_mut() {
                    let slot = ps.bs_read_slot as usize & 1;
                    if b_ps_read && ps.bs_data[slot].b_ps_header_valid == 0 {
                        let cnt = n_bits_left >> 3; // number of remaining bytes
                        for _ in 0..cnt {
                            fdk_read_bits(h_bs, 8);
                        }
                        n_bits_left -= cnt * 8;
                    } else {
                        n_bits_left -= read_ps_data(Some(ps), h_bs, n_bits_left) as i32;
                        b_ps_read = true;
                    }
                }

                // parametric stereo detected, could set channelMode accordingly here.
                // "The usage of this parametric stereo extension to HE-AAC is signalled
                // implicitly in the bitstream. Hence, if an sbr_extension() with
                // bs_extension_id==EXTENSION_ID_PS is found in the SBR part of the bitstream, a
                // decoder supporting the combination of SBR and PS shall operate the PS tool to
                // generate a stereo output signal." source: ISO/IEC 14496-3:2001/FDAM 2:2004(E)
            } else {
                let cnt = n_bits_left >> 3; // number of remaining bytes
                for _ in 0..cnt {
                    fdk_read_bits(h_bs, 8);
                }
                n_bits_left -= cnt * 8;
            }
        }

        if n_bits_left < 0 {
            frame_ok = 0;
        } else {
            // Read fill bits for byte alignment
            fdk_read_bits(h_bs, n_bits_left as u32);
        }
    }

    frame_ok
}

/// Reads the HBE patching mode, oversampling flag and pitch of one channel.
fn read_patching_mode(fd: &mut SbrFrameData, h_bs: &mut FdkBitStream) {
    fd.sbr_patching_mode = fdk_read_bit(h_bs) as u8;
    if fd.sbr_patching_mode == 0 {
        fd.sbr_oversampling_flag = fdk_read_bit(h_bs) as u8;
        if fdk_read_bit(h_bs) != 0 {
            // sbrPitchInBinsFlag
            fd.sbr_pitch_in_bins = fdk_read_bits(h_bs, 7) as u8;
        } else {
            fd.sbr_pitch_in_bins = 0;
        }
    } else {
        fd.sbr_oversampling_flag = 0;
        fd.sbr_pitch_in_bins = 0;
    }
}

/// Sets the legacy patching mode (no HBE).
fn set_legacy_patching(fd: &mut SbrFrameData) {
    fd.sbr_patching_mode = 1;
    fd.sbr_oversampling_flag = 0;
    fd.sbr_pitch_in_bins = 0;
}

/// `sbrGetChannelElement()`: reads the bitstream elements of an SBR channel element (one
/// channel, or a channel pair if `h_frame_data_right` is given). Returns 1 if the frame is ok.
#[allow(clippy::too_many_arguments)]
pub fn sbr_get_channel_element(
    h_header_data: &mut SbrHeaderData,
    h_frame_data_left: &mut SbrFrameData,
    mut h_frame_data_right: Option<&mut SbrFrameData>,
    h_frame_data_left_prev: &SbrPrevFrameData,
    pvc_mode_last: u8,
    h_bs: &mut FdkBitStream,
    h_parametric_stereo_dec: Option<&mut PsDec>,
    flags: u32,
    overlap: i32,
) -> i32 {
    let mut bs_coupling = COUPLING_OFF as u32;
    let n_ch = if h_frame_data_right.is_none() { 1 } else { 2 };

    if flags & (SBRDEC_SYNTAX_USAC | SBRDEC_SYNTAX_RSVD50) == 0 {
        // Reserved bits
        if fdk_read_bits(h_bs, 1) != 0 {
            // bs_data_extra
            fdk_read_bits(h_bs, 4);
            if flags & SBRDEC_SYNTAX_SCAL != 0 || n_ch == 2 {
                fdk_read_bits(h_bs, 4);
            }
        }
    }

    if let Some(right) = h_frame_data_right.as_deref_mut() {
        // Read coupling flag
        bs_coupling = fdk_read_bits(h_bs, 1);
        if bs_coupling != 0 {
            h_frame_data_left.coupling = COUPLING_LEVEL;
            right.coupling = COUPLING_BAL;
        } else {
            h_frame_data_left.coupling = COUPLING_OFF;
            right.coupling = COUPLING_OFF;
        }
    } else {
        if flags & SBRDEC_SYNTAX_SCAL != 0 {
            fdk_read_bits(h_bs, 1); // bs_coupling
        }
        h_frame_data_left.coupling = COUPLING_OFF;
    }

    if flags & (SBRDEC_SYNTAX_USAC | SBRDEC_SYNTAX_RSVD50) != 0 {
        if flags & SBRDEC_USAC_HARMONICSBR != 0 {
            read_patching_mode(h_frame_data_left, h_bs);

            if let Some(right) = h_frame_data_right.as_deref_mut() {
                if bs_coupling != 0 {
                    right.sbr_patching_mode = h_frame_data_left.sbr_patching_mode;
                    right.sbr_oversampling_flag = h_frame_data_left.sbr_oversampling_flag;
                    right.sbr_pitch_in_bins = h_frame_data_left.sbr_pitch_in_bins;
                } else {
                    read_patching_mode(right, h_bs);
                }
            }
        } else {
            if let Some(right) = h_frame_data_right.as_deref_mut() {
                set_legacy_patching(right);
            }
            set_legacy_patching(h_frame_data_left);
        }
    } else {
        if let Some(right) = h_frame_data_right.as_deref_mut() {
            set_legacy_patching(right);
        }
        set_legacy_patching(h_frame_data_left);
    }

    // sbr_grid(): Grid control
    if h_header_data.bs_info.pvc_mode != 0 {
        fdk_assert!(n_ch == 1); // PVC not possible for CPE
        if extract_pvc_frame_info(
            h_bs,
            h_header_data,
            h_frame_data_left,
            h_frame_data_left_prev,
            pvc_mode_last,
            flags,
        ) == 0
        {
            return 0;
        }
    } else if extract_frame_info(h_bs, h_header_data, h_frame_data_left, 1, flags) == 0 {
        return 0;
    }
    if check_frame_info(
        &h_frame_data_left.frame_info,
        h_header_data.number_time_slots as i32,
        overlap,
        h_header_data.time_step as i32,
    ) == 0
    {
        return 0;
    }
    if let Some(right) = h_frame_data_right.as_deref_mut() {
        if h_frame_data_left.coupling != 0 {
            right.frame_info = h_frame_data_left.frame_info;
            right.amp_resolution_current_frame = h_frame_data_left.amp_resolution_current_frame;
        } else {
            if extract_frame_info(h_bs, h_header_data, right, 2, flags) == 0 {
                return 0;
            }

            if check_frame_info(
                &right.frame_info,
                h_header_data.number_time_slots as i32,
                overlap,
                h_header_data.time_step as i32,
            ) == 0
            {
                return 0;
            }
        }
    }

    // sbr_dtdf(): Fetch domain vectors (time or frequency direction for delta-coding)
    sbr_get_direction_control_data(
        h_frame_data_left,
        h_bs,
        flags,
        h_header_data.bs_info.pvc_mode as i32,
    );
    if let Some(right) = h_frame_data_right.as_deref_mut() {
        sbr_get_direction_control_data(right, h_bs, flags, 0);
    }

    // sbr_invf(): nInvfBands = nNfb <= MAX_NOISE_COEFFS
    let n_invf = (h_header_data.freq_band_data.n_invf_bands as usize).min(MAX_INVF_BANDS);
    for i in 0..n_invf {
        h_frame_data_left.sbr_invf_mode[i] = fdk_read_bits(h_bs, 2) as InvfMode;
    }
    if let Some(right) = h_frame_data_right.as_deref_mut() {
        if h_frame_data_left.coupling != 0 {
            right.sbr_invf_mode[..n_invf]
                .copy_from_slice(&h_frame_data_left.sbr_invf_mode[..n_invf]);
        } else {
            for i in 0..n_invf {
                right.sbr_invf_mode[i] = fdk_read_bits(h_bs, 2) as InvfMode;
            }
        }
    }

    match h_frame_data_right.as_deref_mut() {
        None => {
            if h_header_data.bs_info.pvc_mode != 0 {
                let pvc_mode = h_header_data.bs_info.pvc_mode as u32;
                if sbr_get_pvc_envelope(h_header_data, h_frame_data_left, h_bs, flags, pvc_mode)
                    == 0
                {
                    return 0;
                }
            } else if sbr_get_envelope(h_header_data, h_frame_data_left, h_bs, flags) == 0 {
                return 0;
            }

            sbr_get_noise_floor_data(h_header_data, h_frame_data_left, h_bs);
        }
        Some(right) => {
            if h_frame_data_left.coupling != 0 {
                if sbr_get_envelope(h_header_data, h_frame_data_left, h_bs, flags) == 0 {
                    return 0;
                }

                sbr_get_noise_floor_data(h_header_data, h_frame_data_left, h_bs);

                if sbr_get_envelope(h_header_data, right, h_bs, flags) == 0 {
                    return 0;
                }
                sbr_get_noise_floor_data(h_header_data, right, h_bs);
            } else {
                // nCh == 2 && no coupling
                if sbr_get_envelope(h_header_data, h_frame_data_left, h_bs, flags) == 0 {
                    return 0;
                }

                if sbr_get_envelope(h_header_data, right, h_bs, flags) == 0 {
                    return 0;
                }

                sbr_get_noise_floor_data(h_header_data, h_frame_data_left, h_bs);

                sbr_get_noise_floor_data(h_header_data, right, h_bs);
            }
        }
    }

    sbr_get_synthetic_coded_data(h_header_data, h_frame_data_left, h_bs, flags);
    if let Some(right) = h_frame_data_right {
        sbr_get_synthetic_coded_data(h_header_data, right, h_bs, flags);
    }

    if flags & (SBRDEC_SYNTAX_USAC | SBRDEC_SYNTAX_RSVD50) == 0
        && extract_extended_data(h_header_data, h_bs, h_parametric_stereo_dec) == 0
    {
        return 0;
    }

    1
}

/// `sbrGetDirectionControlData()`: reads the direction control data (time or frequency
/// direction of the delta coding of each envelope).
fn sbr_get_direction_control_data(
    h_frame_data: &mut SbrFrameData,
    h_bs: &mut FdkBitStream,
    flags: u32,
    bs_pvc_mode: i32,
) {
    let mut indep_flag = 0;

    if flags & (SBRDEC_SYNTAX_USAC | SBRDEC_SYNTAX_RSVD50) != 0 {
        indep_flag = flags & SBRDEC_USAC_INDEP;
    }

    // nEnvelopes <= MAX_ENVELOPES and nNoiseEnvelopes <= MAX_NOISE_ENVELOPES (checkFrameInfo()).
    if bs_pvc_mode == 0 {
        let mut i = 0usize;
        if indep_flag != 0 {
            h_frame_data.domain_vec[i] = 0;
            i += 1;
        }
        while i < (h_frame_data.frame_info.n_envelopes as usize).min(MAX_ENVELOPES) {
            h_frame_data.domain_vec[i] = fdk_read_bits(h_bs, 1) as u8;
            i += 1;
        }
    }

    let mut i = 0usize;
    if indep_flag != 0 {
        h_frame_data.domain_vec_noise[i] = 0;
        i += 1;
    }
    while i < (h_frame_data.frame_info.n_noise_envelopes as usize).min(MAX_NOISE_ENVELOPES) {
        h_frame_data.domain_vec_noise[i] = fdk_read_bits(h_bs, 1) as u8;
        i += 1;
    }
}

/// `sbrGetNoiseFloorData()`: reads the noise floor level data.
fn sbr_get_noise_floor_data(
    h_header_data: &SbrHeaderData,
    h_frame_data: &mut SbrFrameData,
    h_bs: &mut FdkBitStream,
) {
    let no_noise_bands = h_header_data.freq_band_data.n_nfb as usize;

    let coupling = h_frame_data.coupling;

    // Select huffman codebook depending on coupling mode
    let (hcb_noise, hcb_noise_f, env_data_table_comp_factor): (Huffman, Huffman, i32) =
        if coupling == COUPLING_BAL {
            // "sbr_huffBook_NoiseBalance11F" is sbr_huffBook_EnvBalance11F
            (
                &FDK_SBR_DECODER_SBR_HUFF_BOOK_NOISE_BALANCE11T,
                &FDK_SBR_DECODER_SBR_HUFF_BOOK_ENV_BALANCE11F,
                1,
            )
        } else {
            // "sbr_huffBook_NoiseLevel11F" is sbr_huffBook_EnvLevel11F
            (
                &FDK_SBR_DECODER_SBR_HUFF_BOOK_NOISE_LEVEL11T,
                &FDK_SBR_DECODER_SBR_HUFF_BOOK_ENV_LEVEL11F,
                0,
            )
        };

    // nNoiseEnvelopes <= 2 and nNfb <= 5 (checkFrameInfo(), resetFreqBandTables()): the writes
    // stay inside sbrNoiseFloorLevel[10]; others would be dropped.
    let set = |fd: &mut SbrFrameData, idx: usize, v: i32| {
        if let Some(p) = fd.sbr_noise_floor_level.get_mut(idx) {
            *p = v as FixpSgl;
        }
    };

    // Read raw noise-envelope data
    for i in 0..h_frame_data.frame_info.n_noise_envelopes as usize {
        if h_frame_data.domain_vec_noise.get(i).copied().unwrap_or(0) == 0 {
            if coupling == COUPLING_BAL {
                let v = (fdk_read_bits(h_bs, 5) as i32) << env_data_table_comp_factor;
                set(h_frame_data, i * no_noise_bands, v);
            } else {
                let v = fdk_read_bits(h_bs, 5) as i32;
                set(h_frame_data, i * no_noise_bands, v);
            }

            for j in 1..no_noise_bands {
                let delta = decode_huffman_cw(hcb_noise_f, h_bs);
                set(h_frame_data, i * no_noise_bands + j, delta << env_data_table_comp_factor);
            }
        } else {
            for j in 0..no_noise_bands {
                let delta = decode_huffman_cw(hcb_noise, h_bs);
                set(h_frame_data, i * no_noise_bands + j, delta << env_data_table_comp_factor);
            }
        }
    }
}

/// `sbrGetPvcEnvelope()`: reads the PVC envelope data (the PVC IDs of the 16 time slots).
fn sbr_get_pvc_envelope(
    h_header_data: &mut SbrHeaderData,
    h_frame_data: &mut SbrFrameData,
    h_bs: &mut FdkBitStream,
    flags: u32,
    pvc_mode: u32,
) -> i32 {
    let indep_flag = flags & SBRDEC_USAC_INDEP;

    let mut div_mode = fdk_read_bits(h_bs, PVC_DIVMODE_BITS) as i32;
    let ns_mode = fdk_read_bit(h_bs) as usize;
    fdk_assert!(pvc_mode == 1 || pvc_mode == 2);
    // pvc_mode is 1 or 2 here (values above 2 are rejected as a header error)
    h_frame_data.ns = MAP_NS_MODE2NS[(pvc_mode as usize).wrapping_sub(1).min(1)][ns_mode];
    let pvc_id = &mut h_frame_data.pvc_id;

    if div_mode <= 3 {
        let mut k = 1usize;
        let mut sum_length = 0;

        // special treatment for first time slot k=0
        let reuse_pcv_id = if indep_flag != 0 { 0 } else { fdk_read_bit(h_bs) };
        if reuse_pcv_id != 0 {
            pvc_id[0] = h_header_data.pvc_id_prev;
        } else {
            pvc_id[0] = fdk_read_bits(h_bs, PVC_PVCID_BITS) as u8;
        }

        // other time slots k>0
        for _ in 0..div_mode {
            let mut num_bits = 4;

            if sum_length >= 13 {
                num_bits = 1;
            } else if sum_length >= 11 {
                num_bits = 2;
            } else if sum_length >= 7 {
                num_bits = 3;
            }

            let mut length = fdk_read_bits(h_bs, num_bits) as i32;
            sum_length += length + 1;
            if sum_length >= PVC_NTIMESLOT as i32 {
                return 0; // parse error
            }
            while length > 0 {
                pvc_id[k] = pvc_id[k - 1];
                length -= 1;
                k += 1;
            }
            pvc_id[k] = fdk_read_bits(h_bs, PVC_PVCID_BITS) as u8;
            k += 1;
        }
        while k < 16 {
            pvc_id[k] = pvc_id[k - 1];
            k += 1;
        }
    } else {
        // divMode >= 4
        let mut k = 0usize;

        div_mode -= 4;
        let mut num_grid_info = 2 << div_mode;
        let fixed_length = 8 >> div_mode;
        fdk_assert!(num_grid_info * fixed_length == PVC_NTIMESLOT as i32);

        // special treatment for first time slot k=0
        let grid_info = if indep_flag != 0 { 1 } else { fdk_read_bit(h_bs) };
        if grid_info != 0 {
            pvc_id[k] = fdk_read_bits(h_bs, PVC_PVCID_BITS) as u8;
        } else {
            pvc_id[k] = h_header_data.pvc_id_prev;
        }
        k += 1;
        let mut j = fixed_length - 1;
        while j > 0 {
            pvc_id[k] = pvc_id[k - 1];
            j -= 1;
            k += 1;
        }
        num_grid_info -= 1;

        // other time slots k>0
        while num_grid_info > 0 {
            num_grid_info -= 1;
            let mut j = fixed_length;
            let grid_info = fdk_read_bit(h_bs);
            if grid_info != 0 {
                pvc_id[k] = fdk_read_bits(h_bs, PVC_PVCID_BITS) as u8;
                k += 1;
                j -= 1;
            }
            while j > 0 {
                pvc_id[k] = pvc_id[k - 1];
                j -= 1;
                k += 1;
            }
        }
    }

    h_header_data.pvc_id_prev = pvc_id[PVC_NTIMESLOT - 1];

    // usage of PVC excludes inter-TES tool
    h_frame_data.i_tes_active = 0;

    1
}

/// `sbrGetEnvelope()`: reads the envelope data. Returns 0 if there are too many scale factors.
fn sbr_get_envelope(
    h_header_data: &SbrHeaderData,
    h_frame_data: &mut SbrFrameData,
    h_bs: &mut FdkBitStream,
    flags: u32,
) -> i32 {
    let mut no_band = [0u8; MAX_ENVELOPES];
    let coupling = h_frame_data.coupling;
    let mut amp_res = h_header_data.bs_info.amp_resolution as i32;
    // nEnvelopes <= MAX_ENVELOPES (checkFrameInfo())
    let n_envelopes = (h_frame_data.frame_info.n_envelopes as usize).min(MAX_ENVELOPES);

    h_frame_data.n_scale_factors = 0;

    if h_frame_data.frame_info.frame_class == 0 && n_envelopes == 1 {
        if flags & SBRDEC_ELD_GRID != 0 {
            amp_res = h_frame_data.amp_resolution_current_frame;
        } else {
            amp_res = 0;
        }
    }
    h_frame_data.amp_resolution_current_frame = amp_res;

    // Set number of bits for first value depending on amplitude resolution
    let (start_bits, start_bits_balance) = if amp_res == 1 { (6, 5) } else { (7, 6) };

    // Calculate number of values for each envelope and alltogether
    for i in 0..n_envelopes {
        no_band[i] =
            h_header_data.freq_band_data.n_sfb_at(h_frame_data.frame_info.freq_res[i] as usize);
        h_frame_data.n_scale_factors += no_band[i] as i32;
    }
    if h_frame_data.n_scale_factors > MAX_NUM_ENVELOPE_VALUES as i32 {
        return 0;
    }

    // Select Huffman codebook depending on coupling mode and amplitude resolution
    let (env_data_table_comp_factor, hcb_t, hcb_f): (i32, Huffman, Huffman) =
        if coupling == COUPLING_BAL {
            if amp_res == 0 {
                (
                    1,
                    &FDK_SBR_DECODER_SBR_HUFF_BOOK_ENV_BALANCE10T,
                    &FDK_SBR_DECODER_SBR_HUFF_BOOK_ENV_BALANCE10F,
                )
            } else {
                (
                    1,
                    &FDK_SBR_DECODER_SBR_HUFF_BOOK_ENV_BALANCE11T,
                    &FDK_SBR_DECODER_SBR_HUFF_BOOK_ENV_BALANCE11F,
                )
            }
        } else if amp_res == 0 {
            (
                0,
                &FDK_SBR_DECODER_SBR_HUFF_BOOK_ENV_LEVEL10T,
                &FDK_SBR_DECODER_SBR_HUFF_BOOK_ENV_LEVEL10F,
            )
        } else {
            (
                0,
                &FDK_SBR_DECODER_SBR_HUFF_BOOK_ENV_LEVEL11T,
                &FDK_SBR_DECODER_SBR_HUFF_BOOK_ENV_LEVEL11F,
            )
        };

    h_frame_data.i_tes_active = 0; // disable inter-TES by default

    // Now read raw envelope data
    let mut offset = 0usize;
    for j in 0..n_envelopes {
        let dv = h_frame_data.domain_vec[j];
        if dv == 0 {
            if coupling == COUPLING_BAL {
                h_frame_data.i_envelope[offset] = ((fdk_read_bits(h_bs, start_bits_balance) as i32)
                    << env_data_table_comp_factor)
                    as FixpSgl;
            } else {
                h_frame_data.i_envelope[offset] = fdk_read_bits(h_bs, start_bits) as i32 as FixpSgl;
            }
        }

        // i = 1 - domain_vec[j]: domain_vec is 0 or 1 (one bit, or 0 for USAC independency)
        for i in (1 - dv as i32).max(0) as usize..no_band[j] as usize {
            let delta = if dv == 0 {
                decode_huffman_cw(hcb_f, h_bs)
            } else {
                decode_huffman_cw(hcb_t, h_bs)
            };

            h_frame_data.i_envelope[offset + i] = (delta << env_data_table_comp_factor) as FixpSgl;
        }
        if flags & SBRDEC_SYNTAX_USAC != 0 && flags & SBRDEC_USAC_ITES != 0 {
            let bs_temp_shape = fdk_read_bit(h_bs) as u8;
            fdk_assert!(j < 8);
            h_frame_data.i_tes_active |= bs_temp_shape << j;
            if bs_temp_shape != 0 {
                // bs_inter_temp_shape_mode
                h_frame_data.inter_temp_shape_mode[j] = fdk_read2_bits(h_bs) as u8;
            } else {
                h_frame_data.inter_temp_shape_mode[j] = 0;
            }
        }
        offset += no_band[j] as usize;
    }

    1
}

/// `generateFixFixOnly()`: generates the frame info of the FIXFIX only frame class of the low
/// delay version. Returns 0 on error.
fn generate_fix_fix_only(
    h_sbr_frame_info: &mut FrameInfo,
    tran_pos_internal: i32,
    number_time_slots: i32,
) -> i32 {
    if tran_pos_internal >= number_time_slots {
        return 0;
    }

    let t = tran_pos_internal as usize;
    let p_table: &[i32] = match number_time_slots {
        8 => &FDK_SBR_DECODER_ENVELOPE_TABLE_8[t],
        15 => &FDK_SBR_DECODER_ENVELOPE_TABLE_15[t],
        16 => &FDK_SBR_DECODER_ENVELOPE_TABLE_16[t],
        _ => return 0,
    };

    // look number of envelopes in table
    let n_env = p_table[0] as usize;
    // look up envelope distribution in table
    for i in 1..n_env {
        h_sbr_frame_info.borders[i] = p_table[i + 2] as u8;
    }
    // open and close frame border
    h_sbr_frame_info.borders[0] = 0;
    h_sbr_frame_info.borders[n_env] = number_time_slots as u8;
    h_sbr_frame_info.n_envelopes = n_env as u8;

    // transient idx
    h_sbr_frame_info.tran_env = p_table[1] as i8;
    let tran_idx = h_sbr_frame_info.tran_env as usize;

    // add noise floors
    h_sbr_frame_info.borders_noise[0] = 0;
    h_sbr_frame_info.borders_noise[1] =
        h_sbr_frame_info.borders[if tran_idx != 0 { tran_idx } else { 1 }];
    h_sbr_frame_info.borders_noise[2] = number_time_slots as u8;
    // nEnv is always > 1, so nNoiseEnvelopes is always 2 (IEC 14496-3 4.6.19.3.2)
    h_sbr_frame_info.n_noise_envelopes = 2;

    1
}

/// `extractLowDelayGrid()`: extracts the LowDelaySBR control data from the bitstream. Returns 0
/// on a bitstream error.
fn extract_low_delay_grid(
    h_bit_buf: &mut FdkBitStream,
    h_header_data: &SbrHeaderData,
    h_frame_data: &mut SbrFrameData,
    time_slots: i32,
) -> i32 {
    let number_time_slots = h_header_data.number_time_slots as i32;

    // FIXFIXonly framing case
    h_frame_data.frame_info.frame_class = 0;

    // get the transient position from the bitstream
    let temp = match time_slots {
        // 3bit transient position (temp={0;..;7})
        8 => fdk_read_bits(h_bit_buf, 3) as i32,
        // 4bit transient position (temp={0;..;15})
        16 | 15 => fdk_read_bits(h_bit_buf, 4) as i32,
        _ => return 0,
    };

    // For "case 15" only
    if temp >= time_slots {
        return 0;
    }

    // calculate borders according to the transient position
    if generate_fix_fix_only(&mut h_frame_data.frame_info, temp, number_time_slots) == 0 {
        return 0;
    }

    // decode freq res:
    for k in 0..h_frame_data.frame_info.n_envelopes as usize {
        h_frame_data.frame_info.freq_res[k] = fdk_read_bits(h_bit_buf, 1) as u8;
        // f = F [1 bits]
    }

    1
}

/// `extractPvcFrameInfo()`: extracts the PVC frame information from the bitstream. Returns 0
/// on a bitstream error.
fn extract_pvc_frame_info(
    h_bs: &mut FdkBitStream,
    h_header_data: &SbrHeaderData,
    h_frame_data: &mut SbrFrameData,
    h_prev_frame_data: &SbrPrevFrameData,
    pvc_mode_last: u8,
    _flags: u32,
) -> i32 {
    let p_frame_info = &mut h_frame_data.frame_info;
    let p_prev_frame_info = &h_prev_frame_data.prev_frame_info;
    let bs_noise_position = fdk_read_bits(h_bs, 4); // SBR_PVC_NOISEPOSITION_BITS 4
    let bs_var_len_hf = fdk_read_bit(h_bs);
    p_frame_info.noise_position = bs_noise_position as u8;
    p_frame_info.tran_env = -1;

    // Init for bs_noise_position == 0 in case a parse error is found below.
    p_frame_info.n_envelopes = 1;
    p_frame_info.n_noise_envelopes = 1;
    p_frame_info.freq_res[0] = 0;

    if bs_var_len_hf != 0 {
        // 1 or 3 Bits
        p_frame_info.var_length = (fdk_read_bits(h_bs, 2) + 1) as u8;
        if p_frame_info.var_length > 3 {
            p_frame_info.var_length = 0; // assume bs_var_len_hf == 0 in case of error
            return 0; // reserved value -> parse error
        }
    } else {
        p_frame_info.var_length = 0;
    }

    if bs_noise_position != 0 {
        p_frame_info.n_envelopes = 2;
        p_frame_info.n_noise_envelopes = 2;
        p_frame_info.freq_res = [0; MAX_ENVELOPES];
    }

    // frame border calculation
    if h_header_data.bs_info.pvc_mode > 0 {
        // See "7.5.1.4 HF adjustment of SBR envelope scalefactors" for reference.
        fdk_assert!(p_frame_info.n_envelopes == 1 || p_frame_info.n_envelopes == 2);

        // left timeborder-offset: use the timeborder of prev SBR frame
        if p_prev_frame_info.n_envelopes > 0 {
            // UCHAR - int, stored as UCHAR; prevFrameInfo.nEnvelopes <= 8
            let b = p_prev_frame_info.borders
                [(p_prev_frame_info.n_envelopes as usize).min(MAX_ENVELOPES)];
            p_frame_info.borders[0] = (b as i32 - PVC_NTIMESLOT as i32) as u8;
            fdk_assert!(p_frame_info.borders[0] <= 3);
        } else {
            p_frame_info.borders[0] = 0;
        }

        // right timeborder-offset:
        p_frame_info.borders[p_frame_info.n_envelopes as usize] = 16 + p_frame_info.var_length;

        if p_frame_info.n_envelopes == 2 {
            p_frame_info.borders[1] = p_frame_info.noise_position;
        }

        // Calculation of PVC time borders t_EPVC
        if pvc_mode_last == 0 {
            // there was a legacy SBR frame before this frame => use bs_var_len' for first PVC
            // timeslot
            p_frame_info.pvc_borders[0] = p_frame_info.borders[0];
        } else {
            p_frame_info.pvc_borders[0] = 0;
        }
        if p_frame_info.n_envelopes == 2 {
            p_frame_info.pvc_borders[1] = p_frame_info.borders[1];
        }
        p_frame_info.pvc_borders[p_frame_info.n_envelopes as usize] = 16;

        // calculation of SBR noise-floor time-border vector:
        for i in 0..=p_frame_info.n_noise_envelopes as usize {
            p_frame_info.borders_noise[i] = p_frame_info.borders[i];
        }

        p_frame_info.tran_env = -1; // tranEnv not used
    }
    1
}

/// `extractFrameInfo()`: extracts the frame information (the time grid) from the bitstream.
/// Returns 0 on a bitstream error.
fn extract_frame_info(
    h_bs: &mut FdkBitStream,
    h_header_data: &SbrHeaderData,
    h_frame_data: &mut SbrFrameData,
    _nr_of_channels: u32,
    flags: u32,
) -> i32 {
    let number_time_slots = h_header_data.number_time_slots as i32;
    let mut n_env = 0i32;
    let mut b = 0i32;
    let mut n = 0i32;
    let mut temp = 0i32;
    let frame_class: u8;

    if flags & SBRDEC_ELD_GRID != 0 {
        // CODEC_AACLD (LD+SBR) only uses the normal 0 Grid for non-transient Frames and the
        // LowDelayGrid for transient Frames
        let fc = fdk_read_bits(h_bs, 1) as u8; // frameClass = [1 bit]
        if fc == 1 {
            // if frameClass == 1, extract LowDelaySbrGrid, otherwise extract normal SBR-Grid
            // for FIXIFX
            // extract the AACLD-Sbr-Grid
            h_frame_data.frame_info.frame_class = fc;
            return extract_low_delay_grid(h_bs, h_header_data, h_frame_data, number_time_slots);
        }
        frame_class = fc;
    } else {
        frame_class = fdk_read_bits(h_bs, 2) as u8; // frameClass = C [2 bits]
    }

    match frame_class {
        0 => {
            temp = fdk_read_bits(h_bs, 2) as i32; // E [2 bits ]
            n_env = 1 << temp; // E -> e

            if flags & SBRDEC_ELD_GRID != 0 && n_env == 1 {
                // new ELD Syntax 07-11-09
                h_frame_data.amp_resolution_current_frame = fdk_read_bits(h_bs, 1) as i32;
            }

            let static_freq_res = fdk_read_bits(h_bs, 1);

            if flags & (SBRDEC_SYNTAX_USAC | SBRDEC_SYNTAX_RSVD50) != 0 {
                if n_env > MAX_ENVELOPES_USAC as i32 {
                    return 0;
                }
            } else {
                b = n_env + 1;
            }
            let p_frame_info = &mut h_frame_data.frame_info;
            let table = match (n_env, number_time_slots) {
                (1, 15) => Some(&FDK_SBR_DECODER_SBR_FRAME_INFO1_15),
                (1, 16) => Some(&FDK_SBR_DECODER_SBR_FRAME_INFO1_16),
                (2, 15) => Some(&FDK_SBR_DECODER_SBR_FRAME_INFO2_15),
                (2, 16) => Some(&FDK_SBR_DECODER_SBR_FRAME_INFO2_16),
                (4, 15) => Some(&FDK_SBR_DECODER_SBR_FRAME_INFO4_15),
                (4, 16) => Some(&FDK_SBR_DECODER_SBR_FRAME_INFO4_16),
                (8, 15) => Some(&FDK_SBR_DECODER_SBR_FRAME_INFO8_15),
                (8, 16) => Some(&FDK_SBR_DECODER_SBR_FRAME_INFO8_16),
                _ => {
                    // FDK_ASSERT(0): other numbers of time slots keep the frame info.
                    fdk_assert!(false);
                    None
                }
            };
            if let Some(t) = table {
                *p_frame_info = *t;
            }
            // Apply correct freqRes (High is default)
            if static_freq_res == 0 {
                for i in 0..n_env as usize {
                    p_frame_info.freq_res[i] = 0;
                }
            }
        }
        1 | 2 => {
            temp = fdk_read_bits(h_bs, 2) as i32; // A [2 bits]

            n = fdk_read_bits(h_bs, 2) as i32; // n = N [2 bits]

            n_env = n + 1; // # envelopes
            b = n_env + 1; // # borders
        }
        _ => {}
    }

    let p_frame_info = &mut h_frame_data.frame_info;
    match frame_class {
        1 => {
            // Decode borders:
            p_frame_info.borders[0] = 0; // first border
            let mut border = temp + number_time_slots; // A -> aR
            let mut i = (b - 1) as usize; // frame info index for last border
            p_frame_info.borders[i] = border as u8; // last border

            for _ in 0..n {
                temp = fdk_read_bits(h_bs, 2) as i32; // R [2 bits]
                border -= 2 * temp + 2; // R -> r
                i -= 1;
                p_frame_info.borders[i] = border as u8;
            }

            // Decode pointer:
            let pointer_bits = DFRACT_BITS - 1 - count_leading_bits(n + 1);
            let p = fdk_read_bits(h_bs, pointer_bits as u32) as i32; // p = P [pointer_bits bits]

            if p > n + 1 {
                return 0;
            }

            p_frame_info.tran_env = if p != 0 { (n + 2 - p) as i8 } else { -1 };

            // Decode freq res:
            for k in (0..=n as usize).rev() {
                p_frame_info.freq_res[k] = fdk_read_bits(h_bs, 1) as u8; // f = F [1 bits]
            }

            // Calculate noise floor middle border:
            if p == 0 || p == 1 {
                p_frame_info.borders_noise[1] = p_frame_info.borders[n as usize];
            } else {
                p_frame_info.borders_noise[1] =
                    p_frame_info.borders[p_frame_info.tran_env as usize];
            }
        }
        2 => {
            // Decode borders:
            let mut border = temp; // A -> aL
            p_frame_info.borders[0] = border as u8; // first border

            let mut k = 1usize;
            while k as i32 <= n {
                temp = fdk_read_bits(h_bs, 2) as i32; // R [2 bits]
                border += 2 * temp + 2; // R -> r
                p_frame_info.borders[k] = border as u8;
                k += 1;
            }
            p_frame_info.borders[k] = number_time_slots as u8; // last border

            // Decode pointer:
            let pointer_bits = DFRACT_BITS - 1 - count_leading_bits(n + 1);
            let p = fdk_read_bits(h_bs, pointer_bits as u32) as i32; // p = P [pointer_bits bits]
            if p > n + 1 {
                return 0;
            }

            if p == 0 || p == 1 {
                p_frame_info.tran_env = -1;
            } else {
                p_frame_info.tran_env = (p - 1) as i8;
            }

            // Decode freq res:
            for k in 0..=n as usize {
                p_frame_info.freq_res[k] = fdk_read_bits(h_bs, 1) as u8; // f = F [1 bits]
            }

            // Calculate noise floor middle border:
            match p {
                0 => p_frame_info.borders_noise[1] = p_frame_info.borders[1],
                1 => p_frame_info.borders_noise[1] = p_frame_info.borders[n as usize],
                _ => {
                    p_frame_info.borders_noise[1] =
                        p_frame_info.borders[p_frame_info.tran_env as usize]
                }
            }
        }
        3 => {
            // v_ctrlSignal = [frameClass,aL,aR,nL,nR,v_rL,v_rR,p,v_fLR];

            let a_l = fdk_read_bits(h_bs, 2) as i32; // AL [2 bits], AL -> aL

            let a_r = fdk_read_bits(h_bs, 2) as i32 + number_time_slots; // AR [2 bits], AR -> aR

            let n_l = fdk_read_bits(h_bs, 2) as i32; // nL = NL [2 bits]

            let n_r = fdk_read_bits(h_bs, 2) as i32; // nR = NR [2 bits]

            // Calculate help variables
            // general:
            n_env = n_l + n_r + 1; // # envelopes
            if n_env > MAX_ENVELOPES as i32 {
                return 0;
            }
            b = n_env + 1; // # borders

            // Decode envelopes
            // L-borders:
            let mut border = a_l; // first border
            p_frame_info.borders[0] = border as u8;

            for k in 1..=n_l as usize {
                temp = fdk_read_bits(h_bs, 2) as i32; // R [2 bits]
                border += 2 * temp + 2; // R -> r
                p_frame_info.borders[k] = border as u8;
            }

            // R-borders:
            border = a_r; // last border
            let mut i = n_env as usize;

            p_frame_info.borders[i] = border as u8;

            for _ in 0..n_r {
                temp = fdk_read_bits(h_bs, 2) as i32; // R [2 bits]
                border -= 2 * temp + 2; // R -> r
                i -= 1;
                p_frame_info.borders[i] = border as u8;
            }

            // decode pointer:
            let pointer_bits = DFRACT_BITS - 1 - count_leading_bits(n_l + n_r + 1);
            let p = fdk_read_bits(h_bs, pointer_bits as u32) as i32; // p = P [pointer_bits bits]

            if p > n_l + n_r + 1 {
                return 0;
            }

            p_frame_info.tran_env = if p != 0 { (b - p) as i8 } else { -1 };

            // decode freq res:
            for k in 0..n_env as usize {
                p_frame_info.freq_res[k] = fdk_read_bits(h_bs, 1) as u8; // f = F [1 bits]
            }

            // Decode noise floors
            p_frame_info.borders_noise[0] = a_l as u8;

            if n_env == 1 {
                // 1 noise floor envelope:
                p_frame_info.borders_noise[1] = a_r as u8;
            } else {
                // 2 noise floor envelopes
                if p == 0 || p == 1 {
                    p_frame_info.borders_noise[1] = p_frame_info.borders[n_env as usize - 1];
                } else {
                    p_frame_info.borders_noise[1] =
                        p_frame_info.borders[p_frame_info.tran_env as usize];
                }
                p_frame_info.borders_noise[2] = a_r as u8;
            }
        }
        _ => {}
    }

    // Store number of envelopes, noise floor envelopes and frame class
    p_frame_info.n_envelopes = n_env as u8;

    if n_env == 1 {
        p_frame_info.n_noise_envelopes = 1;
    } else {
        p_frame_info.n_noise_envelopes = 2;
    }

    p_frame_info.frame_class = frame_class;

    if p_frame_info.frame_class == 2 || p_frame_info.frame_class == 1 {
        // calculate noise floor first and last borders:
        p_frame_info.borders_noise[0] = p_frame_info.borders[0];
        p_frame_info.borders_noise[p_frame_info.n_noise_envelopes as usize] =
            p_frame_info.borders[n_env as usize];
    }

    1
}

/// `checkFrameInfo()`: checks that the frame info has reasonable values. Returns 0 on error.
fn check_frame_info(
    p_frame_info: &FrameInfo,
    number_of_time_slots: i32,
    overlap: i32,
    time_step: i32,
) -> i32 {
    let n_envelopes = p_frame_info.n_envelopes as i32;
    let n_noise_envelopes = p_frame_info.n_noise_envelopes as i32;

    if !(1..=MAX_ENVELOPES as i32).contains(&n_envelopes) {
        return 0;
    }

    if n_noise_envelopes > MAX_NOISE_ENVELOPES as i32 {
        return 0;
    }

    let start_pos = p_frame_info.borders[0] as i32;
    let stop_pos = p_frame_info.borders[n_envelopes as usize] as i32;
    let tran_env = p_frame_info.tran_env as i32;
    let mut start_pos_noise = p_frame_info.borders_noise[0] as i32;
    let stop_pos_noise = p_frame_info.borders_noise[n_noise_envelopes as usize] as i32;

    if !(0..=(3 * 4)).contains(&overlap) {
        return 0;
    }
    if !(1..=4).contains(&time_step) {
        return 0;
    }
    let max_pos = number_of_time_slots + (overlap / time_step);

    // Check that the start and stop positions of the frame are reasonable values.
    if start_pos < 0 || start_pos >= stop_pos {
        return 0;
    }
    if start_pos > max_pos - number_of_time_slots {
        // First env. must start in or directly after the overlap buffer
        return 0;
    }
    if stop_pos < number_of_time_slots {
        // One complete frame must be ready for output after processing
        return 0;
    }
    if stop_pos > max_pos {
        return 0;
    }

    // Check that the start border for every envelope is strictly later in time
    for i in 0..n_envelopes as usize {
        if p_frame_info.borders[i] >= p_frame_info.borders[i + 1] {
            return 0;
        }
    }

    // Check that the envelope to be shortened is actually among the envelopes
    if tran_env > n_envelopes {
        return 0;
    }

    // Check the noise borders
    if n_envelopes == 1 && n_noise_envelopes > 1 {
        return 0;
    }

    if start_pos != start_pos_noise || stop_pos != stop_pos_noise {
        return 0;
    }

    // Check that the start border for every noise-envelope is strictly later in time
    for i in 0..n_noise_envelopes as usize {
        if p_frame_info.borders_noise[i] >= p_frame_info.borders_noise[i + 1] {
            return 0;
        }
    }

    // Check that every noise border is the same as an envelope border
    for i in 0..n_noise_envelopes as usize {
        start_pos_noise = p_frame_info.borders_noise[i] as i32;

        let mut j = 0;
        while j < n_envelopes as usize {
            if p_frame_info.borders[j] as i32 == start_pos_noise {
                break;
            }
            j += 1;
        }
        if j == n_envelopes as usize {
            return 0;
        }
    }

    1
}
