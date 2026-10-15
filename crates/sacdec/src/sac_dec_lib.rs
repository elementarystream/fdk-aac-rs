// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libSACdec/include/sac_dec_lib.h, libSACdec/src/sac_dec_lib.cpp.

//! Port of `sac_dec_lib.h` and `sac_dec_lib.cpp`: the API of the MPEG Surround decoder
//! (`mpegSurroundDecoder_*`), used for LD MPEG Surround (ELDv2) and USAC MPS212.
//!
//! # Notes for the AAC decoder glue
//!
//! The functions keep their C parameters, with these differences:
//!
//! - **Handle.** `CMpegSurroundDecoder *` is `Option<&mut CMpegSurroundDecoder>` (`None` for
//!   `NULL`); `mpegSurroundDecoder_Open()` takes `&mut Option<Box<CMpegSurroundDecoder>>`
//!   (`CMpegSurroundDecoder **`). As in C, the first call of `mpegSurroundDecoder_Open()`
//!   (handle `None`) allocates only the outer structure; a second call allocates the spatial
//!   decoder (`mpegSurroundDecoder_IsFullMpegSurroundDecoderInstanceAvailable()`).
//!   `mpegSurroundDecoder_Close()` consumes the handle.
//! - **QMF domain.** The C decoder keeps the pointer `pQmfDomain` that
//!   `mpegSurroundDecoder_Open()` receives (the QMF domain of the AAC decoder) and uses it in
//!   `mpegSurroundDecoder_ConfigureQmfDomain()`, `mpegSurroundDecoder_Init()` and
//!   `mpegSurroundDecoder_Apply()`. The port does not keep the pointer: these functions take
//!   the domain as the parameter `qd` (the AAC decoder passes `&mut self.qmf_domain`, the
//!   domain it passed to `mpegSurroundDecoder_Open()`). The `pQmfDomain` parameter of
//!   `mpegSurroundDecoder_Open()` is kept for the signature and ignored.
//! - **Bitstreams** are `&mut FdkBitStream` (`HANDLE_FDK_BITSTREAM`); the transport decoder's
//!   `cbSsc` callback (`aacDecoder_SscCallback()`) calls
//!   [`mpeg_surround_decoder_config`] with the configuration bitstream.
//! - **`mpegSurroundDecoder_Apply()`**: `input` (the AAC decoder's `workBufferCore2`, the copy
//!   of the downmix for the time interface) and `pTimeData` (the output buffer, also the input
//!   of USAC with `stereoConfigIndex > 1`) are slices; `timeDataSize` is still passed as in C
//!   (the size the AAC decoder computes) and checked against the required size, as in C.
//!   `channelType`, `channelIndices` and `mapDescr` are `Option`s (`NULL`).
//! - **`mpegSurroundDecoder_GetLibInfo()`** takes the `LIB_INFO` array as a slice; the build
//!   date and time are empty strings (a deviation).
//!
//! The parse functions read the MPEG Surround payload of the ancillary data into
//! `mpsData[MPS_DATA_BUFFER_SIZE]` (LD MPEG Surround) and parse it from there: see
//! [`mpeg_surround_decoder_parse`] for the reads beyond that array.

use fdk_aac_rs_fdk::audio::*;
use fdk_aac_rs_fdk::bitbuffer::BitMem;
use fdk_aac_rs_fdk::bitstream::*;
use fdk_aac_rs_fdk::fdk_assert;
use fdk_aac_rs_fdk::qmf::{QMF_FLAG_CLDFB, QMF_FLAG_MPSLDFB};
use fdk_aac_rs_fdk::qmf_domain::FdkQmfDomain;
use fdk_aac_rs_fdk::syslib_channel_map_descr::{fdk_ch_map_descr_is_valid, FdkChannelMapDescr};

use crate::sac_bitdec::*;
use crate::sac_dec::*;
use crate::sac_dec_interface::*;
use crate::sac_process::PcmMpsInput;

pub mod tables;
pub use tables::*;

/// `MPS_DATA_BUFFER_SIZE`
pub const MPS_DATA_BUFFER_SIZE: usize = 2048;

/// `SAC_INPUT_CONFIG`: MPEG Surround input data interface mode.
pub type SacInputConfig = i32;
/// `SAC_INTERFACE_QMF`: use the QMF domain interface for the input downmix audio.
pub const SAC_INTERFACE_QMF: SacInputConfig = 0;
/// `SAC_INTERFACE_TIME`: use the time domain interface for the input downmix audio.
pub const SAC_INTERFACE_TIME: SacInputConfig = 1;
/// `SAC_INTERFACE_AUTO`
pub const SAC_INTERFACE_AUTO: SacInputConfig = 2;

/// `SAC_DEC_OUTPUT_MODE`: MPEG Surround output mode.
pub type SacDecOutputMode = i32;
/// `SACDEC_OUT_MODE_NORMAL`: normal multi channel processing without output restrictions.
pub const SACDEC_OUT_MODE_NORMAL: SacDecOutputMode = 0;
/// `SACDEC_OUT_MODE_BINAURAL`: two channel output with binaural processing.
pub const SACDEC_OUT_MODE_BINAURAL: SacDecOutputMode = 1;
/// `SACDEC_OUT_MODE_STEREO`: always two channel output mode.
pub const SACDEC_OUT_MODE_STEREO: SacDecOutputMode = 2;
/// `SACDEC_OUT_MODE_6CHANNEL`: always process with 5.1 channel output.
pub const SACDEC_OUT_MODE_6CHANNEL: SacDecOutputMode = 3;

/// `SAC_BINAURAL_HRTF_MODEL`
pub type SacBinauralHrtfModel = i32;
pub const SAC_BINAURAL_HRTF_KEMAR: SacBinauralHrtfModel = 0;
pub const SAC_BINAURAL_HRTF_VAST: SacBinauralHrtfModel = 1;
pub const SAC_BINAURAL_HRTF_MPSVT: SacBinauralHrtfModel = 2;
pub const SAC_BINAURAL_SINGLE_HRTFS: SacBinauralHrtfModel = 3;

/// `SAC_INSTANCE_AVAIL`
pub type SacInstanceAvail = i32;
/// `SAC_INSTANCE_NOT_FULL_AVAILABLE`
pub const SAC_INSTANCE_NOT_FULL_AVAILABLE: SacInstanceAvail = 0;
/// `SAC_INSTANCE_FULL_AVAILABLE`
pub const SAC_INSTANCE_FULL_AVAILABLE: SacInstanceAvail = 1;

/// `SACDEC_PARAM`: the dynamic parameters of `mpegSurroundDecoder_SetParam()`.
pub type SacdecParam = i32;
pub const SACDEC_OUTPUT_MODE: SacdecParam = 0x0001;
pub const SACDEC_BLIND_ENABLE: SacdecParam = 0x0002;
pub const SACDEC_PARTIALLY_COMPLEX: SacdecParam = 0x0003;
pub const SACDEC_INTERFACE: SacdecParam = 0x0004;
pub const SACDEC_BS_DELAY: SacdecParam = 0x0005;
pub const SACDEC_BINAURAL_QUALITY: SacdecParam = 0x0102;
pub const SACDEC_BINAURAL_DISTANCE: SacdecParam = 0x0103;
pub const SACDEC_BINAURAL_DIALOG_CLARITY: SacdecParam = 0x0104;
pub const SACDEC_BINAURAL_FRONT_ANGLE: SacdecParam = 0x0105;
pub const SACDEC_BINAURAL_BACK_ANGLE: SacdecParam = 0x0106;
pub const SACDEC_BINAURAL_PRESET: SacdecParam = 0x0107;
pub const SACDEC_BS_INTERRUPTION: SacdecParam = 0x0200;
pub const SACDEC_CLEAR_HISTORY: SacdecParam = 0x0201;
pub const SACDEC_CONCEAL_NUM_KEEP_FRAMES: SacdecParam = 0x0301;
pub const SACDEC_CONCEAL_FADE_OUT_SLOPE_LENGTH: SacdecParam = 0x0302;
pub const SACDEC_CONCEAL_FADE_IN_SLOPE_LENGTH: SacdecParam = 0x0303;
pub const SACDEC_CONCEAL_NUM_RELEASE_FRAMES: SacdecParam = 0x0304;

/// `MPEGS_ANCTYPE`: MPEG Surround data indication.
pub type MpegsAnctype = i32;
/// `MPEGS_ANCTYPE_FRAME`: MPEG Surround frame
pub const MPEGS_ANCTYPE_FRAME: MpegsAnctype = 0;
/// `MPEGS_ANCTYPE_HEADER_AND_FRAME`: MPEG Surround header and MPEG Surround frame
pub const MPEGS_ANCTYPE_HEADER_AND_FRAME: MpegsAnctype = 1;
/// `MPEGS_ANCTYPE_RESERVED_1`
pub const MPEGS_ANCTYPE_RESERVED_1: MpegsAnctype = 2;
/// `MPEGS_ANCTYPE_RESERVED_2`
pub const MPEGS_ANCTYPE_RESERVED_2: MpegsAnctype = 3;

/// `MPEGS_ANCSTARTSTOP`: MPEG Surround data segment indication.
pub type MpegsAncstartstop = i32;
/// `MPEGS_CONTINUE`: the data segment continues a data block
pub const MPEGS_CONTINUE: MpegsAncstartstop = 0;
/// `MPEGS_STOP`: the data segment ends a data block
pub const MPEGS_STOP: MpegsAncstartstop = 1;
/// `MPEGS_START`: the data segment begins a data block
pub const MPEGS_START: MpegsAncstartstop = 2;
/// `MPEGS_START_STOP`: the data segment begins and ends a data block
pub const MPEGS_START_STOP: MpegsAncstartstop = 3;

/// `MPEGS_SYNCSTATE`: MPEG Surround synchronization state (the sync mechanism compares the
/// values).
pub type MpegsSyncstate = i32;
/// `MPEGS_SYNC_LOST`: lost sync because of the current discontinuity
pub const MPEGS_SYNC_LOST: MpegsSyncstate = 0;
/// `MPEGS_SYNC_FOUND`: parsed a valid header and (re)initialization completed
pub const MPEGS_SYNC_FOUND: MpegsSyncstate = 1;
/// `MPEGS_SYNC_COMPLETE`: in sync and continuous, found an independent frame
pub const MPEGS_SYNC_COMPLETE: MpegsSyncstate = 2;

/// `MPEGS_OPMODE`: MPEG Surround operation mode.
pub type MpegsOpmode = i32;
/// `MPEGS_OPMODE_EMM`: Enhanced Matrix Mode (Blind)
pub const MPEGS_OPMODE_EMM: MpegsOpmode = 0;
/// `MPEGS_OPMODE_MPS_PAYLOAD`: Normal, Stereo or Binaural
pub const MPEGS_OPMODE_MPS_PAYLOAD: MpegsOpmode = 1;
/// `MPEGS_OPMODE_NO_MPS_PAYLOAD`: no MPEG Surround payload
pub const MPEGS_OPMODE_NO_MPS_PAYLOAD: MpegsOpmode = 2;

/// `MPEGS_INIT_FLAGS`
pub type MpegsInitFlags = i32;
pub const MPEGS_INIT_OK: MpegsInitFlags = 0x00000000;
pub const MPEGS_INIT_ENFORCE_REINIT: MpegsInitFlags = 0x00000001;
pub const MPEGS_INIT_CHANGE_OUTPUT_MODE: MpegsInitFlags = 0x00000010;
pub const MPEGS_INIT_CHANGE_PARTIALLY_COMPLEX: MpegsInitFlags = 0x00000020;
pub const MPEGS_INIT_CHANGE_TIME_FREQ_INTERFACE: MpegsInitFlags = 0x00000040;
pub const MPEGS_INIT_CHANGE_HEADER: MpegsInitFlags = 0x00000080;
pub const MPEGS_INIT_ERROR_PAYLOAD: MpegsInitFlags = 0x00000100;
pub const MPEGS_INIT_BS_INTERRUPTION: MpegsInitFlags = 0x00001000;
pub const MPEGS_INIT_CLEAR_HISTORY: MpegsInitFlags = 0x00002000;
pub const MPEGS_INIT_CHANGE_CONCEAL_PARAMS: MpegsInitFlags = 0x00100000;
pub const MPEGS_INIT_CHANGE_BYPASS_MODE: MpegsInitFlags = 0x01000000;
pub const MPEGS_INIT_ERROR_ANC_TYPE: MpegsInitFlags = 0x10000000;
pub const MPEGS_INIT_ERROR_ANC_STARTSTOP: MpegsInitFlags = 0x20000000;

/// `struct MpegSurroundDecoder` (`CMpegSurroundDecoder`). `pQmfDomain` is not kept (see the
/// [module documentation](self)).
#[derive(Clone, Debug)]
pub struct CMpegSurroundDecoder {
    /// `mpsData`: buffer for the MPS payload across more than one segment
    pub mps_data: [u8; MPS_DATA_BUFFER_SIZE],
    /// `mpsDataBits`: amount of bits in mpsData
    pub mps_data_bits: i32,
    /// `spatialSpecificConfig[1]`: SSC delay line which is used during decoding
    pub spatial_specific_config: [SpatialSpecificConfig; 1],
    /// `pSpatialDec`
    pub p_spatial_dec: Option<Box<SpatialDec>>,
    /// `spatialSpecificConfigBackup`: SSC used while parsing
    pub spatial_specific_config_backup: SpatialSpecificConfig,
    /// `mpegSurroundDecoderLevel`
    pub mpeg_surround_decoder_level: u8,
    /// `mpegSurroundSscIsGlobalCfg`: the SSC (::spatialSpecificConfig) is an out-of-band
    /// configuration
    pub mpeg_surround_ssc_is_global_cfg: u8,
    /// `mpegSurroundUseTimeInterface`
    pub mpeg_surround_use_time_interface: u8,
    /// `bsFrames[1]`: data read from the SpatialFrame() bitstream element
    pub bs_frames: [SpatialBsFrame; 1],
    /// `llState`: bit stream parser state memory
    pub ll_state: BsLlState,
    /// `bsFrameParse`: current parse frame context index
    pub bs_frame_parse: u8,
    /// `bsFrameDecode`: current decode/apply frame context index
    pub bs_frame_decode: u8,
    /// `bsFrameDelay`: amount of frames delay between parsing and processing
    pub bs_frame_delay: u8,
    /// `mpegSurroundUserParams`
    pub mpeg_surround_user_params: SpatialdecParam,
    /// `upmixType`
    pub upmix_type: SpatialDecUpmixType,
    /// `initFlags[1]`
    pub init_flags: [i32; 1],
    /// `ancStartStopPrev`
    pub anc_start_stop_prev: MpegsAncstartstop,
    /// `fOnSync[1]`
    pub f_on_sync: [MpegsSyncstate; 1],
    /// `decConfig`: initial decoder configuration
    pub dec_config: SpatialDecConfig,
}

impl Default for CMpegSurroundDecoder {
    /// The zero-initialized structure (`FDKcalloc`).
    fn default() -> Self {
        CMpegSurroundDecoder {
            mps_data: [0; MPS_DATA_BUFFER_SIZE],
            mps_data_bits: 0,
            spatial_specific_config: [SpatialSpecificConfig::default()],
            p_spatial_dec: None,
            spatial_specific_config_backup: SpatialSpecificConfig::default(),
            mpeg_surround_decoder_level: 0,
            mpeg_surround_ssc_is_global_cfg: 0,
            mpeg_surround_use_time_interface: 0,
            bs_frames: [SpatialBsFrame::default()],
            ll_state: BsLlState::default(),
            bs_frame_parse: 0,
            bs_frame_decode: 0,
            bs_frame_delay: 0,
            mpeg_surround_user_params: SpatialdecParam::default(),
            upmix_type: 0,
            init_flags: [0],
            anc_start_stop_prev: 0,
            f_on_sync: [0],
            dec_config: SpatialDecConfig::default(),
        }
    }
}

/// The index `bsFrameDecode`/`bsFrameParse` into the one-entry arrays (always 0).
#[inline]
fn fi(i: u8) -> usize {
    (i as usize).min(0)
}

/// `mpegSurroundDecoder_GetNrOfQmfBands(pSsc, sampleRate)`: the number of QMF bands from the
/// sampling frequency (in Hz).
fn mpeg_surround_decoder_get_nr_of_qmf_bands(
    p_ssc: Option<&SpatialSpecificConfig>,
    sample_rate: u32,
) -> i32 {
    let mut sampling_frequency = sample_rate;
    let mut qmf_bands = 64;

    if let Some(p_ssc) = p_ssc {
        match p_ssc.core_codec {
            AOT_USAC => {
                if p_ssc.stereo_config_index == 3 {
                    fdk_assert!(
                        p_ssc.core_sbr_frame_length_index >= 2
                            && p_ssc.core_sbr_frame_length_index <= 4
                    );
                    // Deviation: an index outside of the table keeps 64.
                    if let Some(&b) = MAP_IDX2_QMF_BANDS
                        .get((p_ssc.core_sbr_frame_length_index as usize).wrapping_sub(2))
                    {
                        qmf_bands = b as i32;
                    }
                }
                return qmf_bands;
            }
            _ => {
                sampling_frequency = p_ssc.sampling_freq as u32;
            }
        }
    }

    // number of QMF bands depend on sampling frequency, see FDIS 23003-1:2006 Chapter 6.3.3
    if sampling_frequency < 27713 {
        qmf_bands = 32;
    }
    if sampling_frequency > 55426 {
        qmf_bands = 128;
    }

    qmf_bands
}

/// `mpegSurroundDecoder_CalcInitFlags(pSsc1, pSsc2, upmixTypeFlag, binauralQualityFlag,
/// partiallyComplexFlag, ctrlFlags)`: analyses the init flags.
fn mpeg_surround_decoder_calc_init_flags(
    p_ssc1: &SpatialSpecificConfig,
    p_ssc2: &SpatialSpecificConfig,
    upmix_type_flag: i32,
    binaural_quality_flag: i32,
    partially_complex_flag: i32,
    ctrl_flags: &mut i32,
) -> i32 {
    // Analyse core coder
    if p_ssc1.core_codec != p_ssc2.core_codec {
        *ctrl_flags |= MASK_MPEGS_INIT_ALL_STATES as i32;
        *ctrl_flags |= MASK_MPEGS_INIT_ALL_PARAMS as i32;
    } else {
        // Analyse elements for initialization of space analysis qmf filterbank
        if partially_complex_flag != 0
            || p_ssc1.tree_config != p_ssc2.tree_config
            || p_ssc1.sampling_freq != p_ssc2.sampling_freq
        {
            *ctrl_flags |= MPEGS_INIT_STATES_ANA_QMF_FILTER as i32;
            *ctrl_flags |= MPEGS_INIT_STATES_ANA_HYB_FILTER as i32;
        }

        // Analyse elements for initialization of space synthesis qmf filterbank
        if upmix_type_flag != 0
            || partially_complex_flag != 0
            || p_ssc1.tree_config != p_ssc2.tree_config
            || p_ssc1.sampling_freq != p_ssc2.sampling_freq
            || p_ssc1.bs_fixed_gain_dmx != p_ssc2.bs_fixed_gain_dmx
        {
            *ctrl_flags |= MPEGS_INIT_STATES_SYN_QMF_FILTER as i32;
        }

        // Analyse elements for initialization of decorrelator
        if upmix_type_flag != 0
            || partially_complex_flag != 0
            || p_ssc1.tree_config != p_ssc2.tree_config
            || p_ssc1.sampling_freq != p_ssc2.sampling_freq
            || p_ssc1.decorr_config != p_ssc2.decorr_config
        {
            *ctrl_flags |= MPEGS_INIT_STATES_DECORRELATOR as i32;
        }

        // Analyse elements for initialization of m1 and m2 calculation
        if upmix_type_flag != 0
            || binaural_quality_flag != 0
            || p_ssc1.tree_config != p_ssc2.tree_config
            || p_ssc1.sampling_freq != p_ssc2.sampling_freq
        {
            *ctrl_flags |= MPEGS_INIT_STATES_M1M2 as i32;
        }

        // Analyse elements for initialization of GES
        if upmix_type_flag != 0
            || p_ssc1.tree_config != p_ssc2.tree_config
            || p_ssc1.temp_shape_config != p_ssc2.temp_shape_config
        {
            *ctrl_flags |= MPEGS_INIT_STATES_GES as i32;
        }

        // Analyse elements for initialization of FDreverb
        if upmix_type_flag != 0
            || binaural_quality_flag != 0
            || partially_complex_flag != 0
            || p_ssc1.sampling_freq != p_ssc2.sampling_freq
            || p_ssc1.n_time_slots != p_ssc2.n_time_slots
        {
            *ctrl_flags |= MPEGS_INIT_STATES_REVERB as i32;
        }

        // Reset previous frame data whenever the config changes
        if *ctrl_flags & MPEGS_INIT_CONFIG as i32 != 0 {
            *ctrl_flags |= MPEGS_INIT_STATES_PARAM as i32;
        }
    }

    MPS_OK
}

/// `updateMpegSurroundDecoderStatus(pMpegSurroundDecoder, initFlags, fOnSync,
/// ancStartStopPrev)`: resets the MPEG Surround status info.
fn update_mpeg_surround_decoder_status(
    d: &mut CMpegSurroundDecoder,
    init_flags: i32,
    f_on_sync: MpegsSyncstate,
    anc_start_stop_prev: MpegsAncstartstop,
) {
    let dec = fi(d.bs_frame_decode);
    d.init_flags[dec] |= init_flags;
    if d.mpeg_surround_ssc_is_global_cfg != 0
        && d.f_on_sync[dec] >= MPEGS_SYNC_FOUND
        && f_on_sync < MPEGS_SYNC_FOUND
    {
        d.f_on_sync[dec] = MPEGS_SYNC_FOUND;
    } else {
        d.f_on_sync[dec] = f_on_sync;
    }
    d.anc_start_stop_prev = anc_start_stop_prev;
}

/// `mpegSurroundDecoder_IsFullMpegSurroundDecoderInstanceAvailable(pMpegSurroundDecoder)`
pub fn mpeg_surround_decoder_is_full_mpeg_surround_decoder_instance_available(
    p_mpeg_surround_decoder: &CMpegSurroundDecoder,
) -> SacInstanceAvail {
    if p_mpeg_surround_decoder.p_spatial_dec.is_some() {
        SAC_INSTANCE_FULL_AVAILABLE
    } else {
        SAC_INSTANCE_NOT_FULL_AVAILABLE
    }
}

/// `mpegSurroundDecoder_Open(pMpegSurroundDecoder, stereoConfigIndex, pQmfDomain)`: opens one
/// instance of the MPEG Surround decoder. The QMF domain is not kept (see the
/// [module documentation](self)).
pub fn mpeg_surround_decoder_open(
    p_mpeg_surround_decoder: &mut Option<Box<CMpegSurroundDecoder>>,
    stereo_config_index: i32,
    _p_qmf_domain: Option<&mut FdkQmfDomain>,
) -> SacdecError {
    mpeg_surround_decoder_create(p_mpeg_surround_decoder, stereo_config_index)
}

/// `check_UParam_Build_DecConfig(pUserParams, pDecConfig, pSsc, pUpmixType)`: checks whether
/// the user parameters, the decoder configuration and the SSC are valid and supported; may set
/// the upmix type.
fn check_uparam_build_dec_config(
    p_user_params: &SpatialdecParam,
    p_dec_config: &SpatialDecConfig,
    p_ssc: &SpatialSpecificConfig,
    p_upmix_type: &mut SpatialDecUpmixType,
) -> SacdecError {
    // checks if implementation can handle the Ssc
    let (dmx_channels, out_channels) = match p_ssc.tree_config {
        SPATIALDEC_MODE_RSVD7 => (1, 2), // 212
        _ => return MPS_UNSUPPORTED_CONFIG,
    };

    // Analyse pDecConfig params
    match p_dec_config.binaural_mode {
        BINAURAL_NONE => {}
        _ => return MPS_UNSUPPORTED_CONFIG,
    }

    match p_dec_config.decoder_mode {
        EXT_HQ_ONLY => {}
        _ => return MPS_UNSUPPORTED_CONFIG,
    }

    let max_num_out_channels = match p_dec_config.max_num_output_channels {
        OUTPUT_CHANNELS_DEFAULT => {
            // No special restrictions -> Get the level restriction:
            match p_dec_config.decoder_level {
                DECODER_LEVEL_0 => 2,
                _ => return MPS_UNSUPPORTED_CONFIG,
            }
        }
        OUTPUT_CHANNELS_2_0 => 2,
        _ => return MPS_UNSUPPORTED_CONFIG,
    };

    // check if we can handle user params
    if p_user_params.blind_enable == 1 {
        return MPS_UNSUPPORTED_CONFIG;
    }
    {
        match p_user_params.output_mode as i32 {
            SACDEC_OUT_MODE_NORMAL => {
                if max_num_out_channels >= out_channels {
                    *p_upmix_type = UPMIX_TYPE_NORMAL;
                } else {
                    *p_upmix_type = UPMIX_TYPE_BYPASS;
                }
            }
            SACDEC_OUT_MODE_STEREO => {
                if dmx_channels == 1 {
                    if out_channels == 2 {
                        *p_upmix_type = UPMIX_TYPE_NORMAL;
                    }
                } else {
                    *p_upmix_type = UPMIX_TYPE_BYPASS;
                }
            }
            SACDEC_OUT_MODE_6CHANNEL => {
                if out_channels > 6 {
                    *p_upmix_type = UPMIX_TYPE_BYPASS;
                } else {
                    *p_upmix_type = UPMIX_TYPE_NORMAL;
                }
            }
            _ => return MPS_UNSUPPORTED_CONFIG,
        }
    }

    MPS_OK
}

/// `initMpegSurroundDecoder(pMpegSurroundDecoder)`
///
/// Deviation: without a spatial decoder (no defined result in C) the port returns
/// `MPS_INVALID_HANDLE`.
fn init_mpeg_surround_decoder(d: &mut CMpegSurroundDecoder, qd: &mut FdkQmfDomain) -> SacdecError {
    let mut err;
    let mut init_flags = MPEGS_INIT_NONE as i32;
    let upmix_type_curr = d.upmix_type;
    let dec = fi(d.bs_frame_decode);

    let mut init_flags_dec = d.init_flags[dec];

    'bail: {
        if d.spatial_specific_config_backup.core_codec != AOT_USAC {
            // here we check if we have a valid Ssc
            err = ssc_parse_check(&d.spatial_specific_config_backup);
            if err != MPS_OK {
                break 'bail;
            }
        }

        // here we check if Ssc matches build; also check UParams and DecConfig
        // if desired upmixType is changes
        err = check_uparam_build_dec_config(
            &d.mpeg_surround_user_params,
            &d.dec_config,
            &d.spatial_specific_config_backup,
            &mut d.upmix_type,
        );
        if err != MPS_OK {
            break 'bail;
        }

        // init config
        if init_flags_dec & MPEGS_INIT_CHANGE_HEADER != 0 {
            init_flags |= MPEGS_INIT_CONFIG as i32;
        }
        // init all states
        if init_flags_dec & MPEGS_INIT_CLEAR_HISTORY != 0 {
            init_flags |= MASK_MPEGS_INIT_ALL_STATES as i32;
        }
        if init_flags_dec & MPEGS_INIT_CHANGE_CONCEAL_PARAMS != 0 {
            init_flags |= MPEGS_INIT_PARAMS_ERROR_CONCEALMENT as i32;
        }

        if init_flags_dec & MPEGS_INIT_ENFORCE_REINIT != 0 {
            // init all states
            init_flags |= MASK_MPEGS_INIT_ALL_STATES as i32;
            init_flags |= MASK_MPEGS_INIT_ALL_PARAMS as i32;
        } else {
            // analyse states which have to be initialized
            mpeg_surround_decoder_calc_init_flags(
                &d.spatial_specific_config[dec],
                &d.spatial_specific_config_backup,
                (upmix_type_curr != d.upmix_type) as i32, // upmixType changed
                0,
                (init_flags_dec & MPEGS_INIT_CHANGE_PARTIALLY_COMPLEX != 0) as i32,
                &mut init_flags,
            );
        }

        {
            d.spatial_specific_config[dec] = d.spatial_specific_config_backup;
            let nr_of_qmf_bands = mpeg_surround_decoder_get_nr_of_qmf_bands(
                Some(&d.spatial_specific_config[dec]),
                d.spatial_specific_config[dec].sampling_freq as u32,
            );
            let CMpegSurroundDecoder {
                p_spatial_dec,
                bs_frames,
                spatial_specific_config,
                upmix_type,
                mpeg_surround_user_params,
                ..
            } = d;
            let Some(sd) = p_spatial_dec.as_deref_mut() else {
                err = MPS_INVALID_HANDLE;
                break 'bail;
            };
            err = fdk_spatial_dec_init(
                sd,
                qd,
                &mut bs_frames[dec],
                &mut spatial_specific_config[dec],
                nr_of_qmf_bands,
                *upmix_type,
                mpeg_surround_user_params,
                init_flags as u32,
            );

            if err != MPS_OK {
                break 'bail;
            }

            // Signal that we got a header and can go on decoding
            if err == MPS_OK {
                init_flags_dec = MPEGS_INIT_OK;
                {
                    d.f_on_sync[dec] = MPEGS_SYNC_FOUND;
                }
            }
        }
    }

    d.init_flags[dec] = init_flags_dec;
    err
}

/// `mpegSurroundDecoder_Init(pMpegSurroundDecoder)`: initializes the decoder if it has pending
/// init flags. `qd` is the QMF domain of the decoder.
pub fn mpeg_surround_decoder_init(
    p_mpeg_surround_decoder: &mut CMpegSurroundDecoder,
    qd: &mut FdkQmfDomain,
) -> SacdecError {
    let mut err = MPS_OK;

    if p_mpeg_surround_decoder.init_flags[fi(p_mpeg_surround_decoder.bs_frame_decode)] != 0 {
        err = init_mpeg_surround_decoder(p_mpeg_surround_decoder, qd);
    }

    err
}

/// `mpegSurroundDecoder_Create(pMpegSurroundDecoder, stereoConfigIndex, pQmfDomain)`
fn mpeg_surround_decoder_create(
    p_mpeg_surround_decoder: &mut Option<Box<CMpegSurroundDecoder>>,
    stereo_config_index: i32,
) -> SacdecError {
    let err = MPS_OK;

    // decoderLevel  decoderMode  maxNumOutputChannels  binauralMode
    let dec_config = DEC_CONFIG;

    let Some(sac_dec) = p_mpeg_surround_decoder.as_deref_mut() else {
        let mut d = Box::<CMpegSurroundDecoder>::default();
        for i in 0..1 {
            spatial_dec_create_bs_frame(&mut d.bs_frames[i]);
        }

        d.bs_frame_delay = 1;
        d.bs_frame_parse = 0;
        d.bs_frame_decode = 0;

        *p_mpeg_surround_decoder = Some(d);
        return err;
    };

    if sac_dec.p_spatial_dec.is_none() {
        match fdk_spatial_dec_open(&dec_config, stereo_config_index) {
            Some(s) => sac_dec.p_spatial_dec = Some(s),
            None => {
                // bail: (unreachable: the allocation of the port cannot fail)
                let d = p_mpeg_surround_decoder.take();
                mpeg_surround_decoder_close(d);
                return MPS_OUTOFMEMORY;
            }
        }
    }

    // self->pQmfDomain = sacDec->pQmfDomain (not kept, see the module documentation)

    // default parameter set
    sac_dec.mpeg_surround_user_params.output_mode = SACDEC_OUT_MODE_NORMAL as u8;
    sac_dec.mpeg_surround_user_params.blind_enable = 0;
    sac_dec.mpeg_surround_user_params.bypass_mode = 0;
    sac_dec.mpeg_surround_user_params.conceal_method = 1;
    sac_dec.mpeg_surround_user_params.conceal_num_keep_frames = 10;
    sac_dec.mpeg_surround_user_params.conceal_fade_out_slope_length = 5;
    sac_dec.mpeg_surround_user_params.conceal_fade_in_slope_length = 5;
    sac_dec.mpeg_surround_user_params.conceal_num_release_frames = 3;
    sac_dec.mpeg_surround_ssc_is_global_cfg = 0;
    sac_dec.mpeg_surround_use_time_interface = 1;
    sac_dec.mpeg_surround_decoder_level = dec_config.decoder_level as u8;

    sac_dec.upmix_type = UPMIX_TYPE_NORMAL;

    // signalize spatial decoder re-initalization
    update_mpeg_surround_decoder_status(
        sac_dec,
        MPEGS_INIT_ENFORCE_REINIT,
        MPEGS_SYNC_LOST,
        MPEGS_STOP,
    );

    // return decoder instance
    sac_dec.dec_config = dec_config;

    if let Some(sd) = sac_dec.p_spatial_dec.as_deref_mut() {
        spatial_dec_init_parser_context(sd);
    }

    err
}

/// `mpegSurroundDecoder_Config(pMpegSurroundDecoder, hBs, coreCodec, samplingRate, frameSize,
/// numChannels, stereoConfigIndex, coreSbrFrameLengthIndex, configBytes, configMode,
/// configChanged)`: reads and parses the SpatialSpecificConfig (the `cbSsc` callback of the
/// transport decoder).
pub fn mpeg_surround_decoder_config(
    p_mpeg_surround_decoder: &mut CMpegSurroundDecoder,
    h_bs: &mut FdkBitStream,
    core_codec: AudioObjectType,
    sampling_rate: i32,
    frame_size: i32,
    num_channels: i32,
    stereo_config_index: i32,
    core_sbr_frame_length_index: i32,
    config_bytes: i32,
    config_mode: u8,
    config_changed: &mut u8,
) -> SacdecError {
    let mut err;
    let n_input_channels;
    let mut spatial_specific_config = SpatialSpecificConfig::default();
    let d = p_mpeg_surround_decoder;
    let use_tmp = config_mode == AC_CM_DET_CFG_CHANGE;

    match core_codec {
        AOT_DRM_USAC | AOT_USAC => {
            // In config detection mode write spatial specific config parameters into
            // temporarily allocated structure
            let target = if use_tmp {
                &mut spatial_specific_config
            } else {
                &mut d.spatial_specific_config_backup
            };
            err = spatial_dec_parse_mps212_config(
                h_bs,
                target,
                sampling_rate,
                core_codec,
                stereo_config_index,
                core_sbr_frame_length_index,
            );
            n_input_channels = target.n_input_channels;
            if err == MPS_OK && num_channels != n_input_channels {
                err = MPS_PARSE_ERROR;
            }
        }
        AOT_ER_AAC_ELD | AOT_ER_AAC_LD => {
            // In config detection mode write spatial specific config parameters into
            // temporarily allocated structure
            let target = if use_tmp {
                &mut spatial_specific_config
            } else {
                &mut d.spatial_specific_config_backup
            };
            err = spatial_dec_parse_specific_config(h_bs, target, config_bytes, core_codec);
            n_input_channels = target.n_input_channels;
            // check number of channels for channel_configuration > 0
            if err == MPS_OK && num_channels > 0 && num_channels != n_input_channels {
                err = MPS_PARSE_ERROR;
            }
        }
        _ => {
            err = MPS_UNSUPPORTED_FORMAT;
        }
    }

    if err != MPS_OK {
        return err;
    }

    {
        let p_ssc =
            if use_tmp { &spatial_specific_config } else { &d.spatial_specific_config_backup };
        err = ssc_check_out_of_band(p_ssc, core_codec, sampling_rate, frame_size);
    }

    if err != MPS_OK {
        return err;
    }

    if config_mode & AC_CM_DET_CFG_CHANGE != 0 {
        return err;
    }

    if config_mode & AC_CM_ALLOC_MEM != 0 && *config_changed != 0 {
        // mpegSurroundDecoder_Open(&pMpegSurroundDecoder, stereoConfigIndex, NULL) on an
        // existing decoder: allocates the spatial decoder if needed and resets the parameters.
        let mut opt = Some(Box::new(core::mem::take(d)));
        err = mpeg_surround_decoder_create(&mut opt, stereo_config_index);
        match opt {
            Some(b) => *d = *b,
            None => {
                // C closes the decoder here (unreachable: the allocation cannot fail).
                *d = CMpegSurroundDecoder::default();
            }
        }
        if err != MPS_OK {
            return err;
        }
    }

    {
        let parse = fi(d.bs_frame_parse);
        if fdk_spatial_dec_compare_spatial_specific_config_header(
            &d.spatial_specific_config_backup,
            &d.spatial_specific_config[parse],
        ) != 0
        {
            d.init_flags[parse] |= MPEGS_INIT_CHANGE_HEADER;
            // Error resilience code
            let Some(sd) = d.p_spatial_dec.as_deref_mut() else {
                return MPS_NOTOK;
            };
            spatial_dec_init_parser_context(sd);
            // pConfigCurrent = &spatialSpecificConfig[bsFrameDecode]
            sd.p_config_current = true;
        }
    }

    if err == MPS_OK {
        // We got a valid out-of-band configuration so label it accordingly.
        d.mpeg_surround_ssc_is_global_cfg = 1;
    }

    err
}

/// `mpegSurroundOperationMode(pMpegSurroundDecoder, mpsDataBits)`: determines the operation
/// mode.
fn mpeg_surround_operation_mode(d: &mut CMpegSurroundDecoder, mps_data_bits: i32) -> MpegsOpmode {
    if mps_data_bits > 0 && d.mpeg_surround_user_params.blind_enable == 0 {
        MPEGS_OPMODE_MPS_PAYLOAD // Mode: Normal, Stereo or Binaural
    } else {
        // Mode: No MPEG Surround Payload
        update_mpeg_surround_decoder_status(
            d,
            MPEGS_INIT_ERROR_PAYLOAD,
            MPEGS_SYNC_LOST,
            MPEGS_STOP,
        );
        MPEGS_OPMODE_NO_MPS_PAYLOAD
    }
}

/// `sscParseCheck(pSsc)`: checks the SSC for parse errors.
fn ssc_parse_check(p_ssc: &SpatialSpecificConfig) -> SacdecError {
    if p_ssc.sampling_freq > 96000 {
        return MPS_PARSE_ERROR;
    }
    if p_ssc.sampling_freq < 8000 {
        return MPS_PARSE_ERROR;
    }

    if p_ssc.tree_config < 0 || p_ssc.tree_config > 7 {
        return MPS_PARSE_ERROR;
    }

    if p_ssc.quant_mode < 0 || p_ssc.quant_mode > 2 {
        return MPS_PARSE_ERROR;
    }

    // now we are sure there were no parsing errors
    MPS_OK
}

/// `checkTimeSlots(frameLength, qmfBands, timeSlots)`: the MPS frame length must be a
/// multiple of the core coder frame length (ISO/IEC 23003-1, Table 4A).
pub fn check_time_slots(frame_length: i32, qmf_bands: i32, time_slots: i32) -> SacdecError {
    let max_frame_length = if qmf_bands == 64 {
        // normal MPEG Surround
        match frame_length {
            960 | 1920 => 3840,
            1024 | 2048 => 4096,
            512 | 1152 => 4608,
            _ => return MPS_PARSE_ERROR,
        }
    } else if qmf_bands == 32 {
        // downsampled MPEG Surround
        match frame_length {
            960 | 1920 => 1920,
            512 | 1024 | 2048 => 2048,
            1152 => 2304,
            _ => return MPS_PARSE_ERROR,
        }
    } else if qmf_bands == 128 {
        // upsampled MPEG Surround
        match frame_length {
            1920 => 7680,
            1024 => 9216,
            2048 => 8192,
            // no support for upsampled MPEG Surround
            _ => return MPS_PARSE_ERROR,
        }
    } else {
        return MPS_PARSE_ERROR;
    };

    let mut len = frame_length;

    while len <= max_frame_length {
        if len == time_slots.wrapping_mul(qmf_bands) {
            return MPS_OK;
        }
        len += frame_length;
    }

    MPS_PARSE_ERROR
}

/// `sscCheckInBand(pSsc, frameLength, sampleRate)`: checks the in-band SSC for consistency.
fn ssc_check_in_band(
    p_ssc: &SpatialSpecificConfig,
    frame_length: i32,
    sample_rate: i32,
) -> SacdecError {
    let mut err = MPS_OK;

    // check ssc for parse errors
    if ssc_parse_check(p_ssc) != MPS_OK {
        err = MPS_PARSE_ERROR;
    }

    // core fs and mps fs must match
    if p_ssc.sampling_freq != sample_rate {
        err = MPS_PARSE_ERROR; // MPEGSDEC_SSC_PARSE_ERROR
    }

    let qmf_bands =
        mpeg_surround_decoder_get_nr_of_qmf_bands(Some(p_ssc), p_ssc.sampling_freq as u32);

    if check_time_slots(frame_length, qmf_bands, p_ssc.n_time_slots) != MPS_OK {
        err = MPS_PARSE_ERROR;
    }

    err
}

/// `mpegSurroundDecoder_ConfigureQmfDomain(pMpegSurroundDecoder, sac_dec_interface,
/// coreSamplingRate, coreCodec)`: sets the requested QMF domain parameters of the decoder.
///
/// Deviation: without a spatial decoder (no defined result in C) the port returns
/// `MPS_INVALID_HANDLE`.
pub fn mpeg_surround_decoder_configure_qmf_domain(
    p_mpeg_surround_decoder: Option<&mut CMpegSurroundDecoder>,
    qd: &mut FdkQmfDomain,
    sac_dec_interface: SacInputConfig,
    core_sampling_rate: u32,
    core_codec: AudioObjectType,
) -> SacdecError {
    let err = MPS_OK;

    let Some(d) = p_mpeg_surround_decoder else {
        return MPS_INVALID_HANDLE;
    };

    fdk_assert!(d.p_spatial_dec.is_some());
    let Some(sd) = d.p_spatial_dec.as_deref() else {
        return MPS_INVALID_HANDLE;
    };

    let p_gc = &mut qd.global_conf;
    if d.mpeg_surround_ssc_is_global_cfg != 0 {
        let p_ssc = &d.spatial_specific_config_backup;
        if sac_dec_interface == SAC_INTERFACE_TIME {
            // For SAC_INTERFACE_QMF these parameters are set by SBR.
            // coreSamplingRate == outputSamplingRate for SAC_INTERFACE_TIME
            p_gc.n_bands_analysis_requested =
                mpeg_surround_decoder_get_nr_of_qmf_bands(Some(p_ssc), core_sampling_rate) as u8;
            p_gc.n_bands_synthesis_requested = p_gc.n_bands_analysis_requested as u16;
            p_gc.n_input_channels_requested =
                f_max_u32(p_ssc.n_input_channels as u32, p_gc.n_input_channels_requested as u32)
                    as u8;
        }
        p_gc.n_output_channels_requested =
            f_max_u32(p_ssc.n_output_channels as u32, p_gc.n_output_channels_requested as u32)
                as u8;
    } else {
        if sac_dec_interface == SAC_INTERFACE_TIME {
            // For SAC_INTERFACE_QMF these parameters are set by SBR.
            // coreSamplingRate == outputSamplingRate for SAC_INTERFACE_TIME
            p_gc.n_bands_analysis_requested =
                mpeg_surround_decoder_get_nr_of_qmf_bands(None, core_sampling_rate) as u8;
            p_gc.n_bands_synthesis_requested = p_gc.n_bands_analysis_requested as u16;
            p_gc.n_input_channels_requested = sd.create_params.max_num_input_channels as u8;
        }
        p_gc.n_output_channels_requested = sd.create_params.max_num_output_channels as u8;
    }
    p_gc.n_qmf_proc_bands_requested = 64;
    p_gc.n_qmf_proc_channels_requested =
        f_min(p_gc.n_input_channels_requested as i32, sd.create_params.max_num_input_channels)
            as u8;

    if core_codec == AOT_ER_AAC_ELD {
        p_gc.flags_requested |= QMF_FLAG_MPSLDFB;
        p_gc.flags_requested &= !QMF_FLAG_CLDFB;
    }

    err
}

/// `fMax(UINT, UINT)`
#[inline]
fn f_max_u32(a: u32, b: u32) -> u32 {
    if a > b {
        a
    } else {
        b
    }
}

/// `fMin(INT, INT)`
#[inline]
fn f_min(a: i32, b: i32) -> i32 {
    if a < b {
        a
    } else {
        b
    }
}

/// `sscCheckOutOfBand(pSsc, coreCodec, sampleRate, frameSize)`: checks an out-of-band config.
fn ssc_check_out_of_band(
    p_ssc: &SpatialSpecificConfig,
    core_codec: i32,
    sample_rate: i32,
    frame_size: i32,
) -> SacdecError {
    // check ssc for parse errors
    if ssc_parse_check(p_ssc) != MPS_OK {
        return MPS_PARSE_ERROR;
    }

    match core_codec {
        AOT_USAC | AOT_DRM_USAC => {
            // ISO/IEC 23003-1:2007(E), Chapter 6.3.3, Support for lower and higher sampling
            // frequencies
            if p_ssc.sampling_freq >= 55426 {
                return MPS_PARSE_ERROR;
            }
        }
        AOT_ER_AAC_LD | AOT_ER_AAC_ELD => {
            // core fs and mps fs must match
            if p_ssc.sampling_freq != sample_rate {
                return MPS_PARSE_ERROR;
            }

            // ISO/IEC 14496-3:2009 FDAM 3: Chapter 1.5.2.3, Levels for the Low Delay AAC v2
            // profile
            if p_ssc.sampling_freq > 48000 {
                return MPS_PARSE_ERROR;
            }

            let qmf_bands =
                mpeg_surround_decoder_get_nr_of_qmf_bands(Some(p_ssc), p_ssc.sampling_freq as u32);
            let ok = match frame_size {
                480 => qmf_bands == 32 && p_ssc.n_time_slots == 15,
                960 => qmf_bands == 64 && p_ssc.n_time_slots == 15,
                512 => {
                    (qmf_bands == 32 && p_ssc.n_time_slots == 16)
                        || (qmf_bands == 64 && p_ssc.n_time_slots == 8)
                }
                1024 => qmf_bands == 64 && p_ssc.n_time_slots == 16,
                _ => false,
            };
            if !ok {
                return MPS_PARSE_ERROR;
            }
        }
        _ => return MPS_PARSE_ERROR,
    }

    MPS_OK
}

/// `mpegSurroundDecoder_ParseNoHeader(pMpegSurroundDecoder, hBs, pMpsDataBits,
/// fGlobalIndependencyFlag)`: parses MPEG Surround data without header (USAC MPS212).
pub fn mpeg_surround_decoder_parse_no_header(
    p_mpeg_surround_decoder: Option<&mut CMpegSurroundDecoder>,
    h_bs: Option<&mut FdkBitStream>,
    p_mps_data_bits: &mut i32,
    f_global_independency_flag: i32,
) -> i32 {
    let mut err;

    let (Some(d), Some(h_bs)) = (p_mpeg_surround_decoder, h_bs) else {
        return MPS_INVALID_HANDLE;
    };

    let parse = fi(d.bs_frame_parse);

    let bits_avail = fdk_get_valid_bits(h_bs) as i32;

    // First spatial specific config is parsed into spatialSpecificConfigBackup, second
    // spatialSpecificConfigBackup is copied into spatialSpecificConfig[bsFrameDecode]
    if d.init_flags[parse] != 0 {
        d.spatial_specific_config[parse] = d.spatial_specific_config_backup;
        d.f_on_sync[parse] = MPEGS_SYNC_FOUND;
    }

    if bits_avail <= 0 {
        err = MPS_PARSE_ERROR;
    } else {
        let CMpegSurroundDecoder {
            p_spatial_dec,
            bs_frames,
            ll_state,
            spatial_specific_config,
            upmix_type,
            ..
        } = d;
        // Deviation: without a spatial decoder (no defined result in C) the port returns
        // MPS_INVALID_HANDLE.
        let Some(sd) = p_spatial_dec.as_deref_mut() else {
            return MPS_INVALID_HANDLE;
        };
        err = spatial_dec_parse_frame_data(
            sd,
            &mut bs_frames[parse],
            ll_state,
            h_bs,
            &spatial_specific_config[parse],
            *upmix_type,
            f_global_independency_flag,
        );
        if err == MPS_OK {
            bs_frames[parse].new_bs_data = 1;
        }
    }

    let num_sac_bits = bits_avail.wrapping_sub(fdk_get_valid_bits(h_bs) as i32);

    if num_sac_bits > bits_avail {
        d.bs_frames[parse].new_bs_data = 0;
        err = MPS_PARSE_ERROR;
    }

    *p_mps_data_bits = p_mps_data_bits.wrapping_sub(num_sac_bits);

    err
}

/// `isValidAncType(pMpegSurroundDecoder, ancType)`
fn is_valid_anc_type(d: &mut CMpegSurroundDecoder, anc_type: i32) -> i32 {
    let mut ret = 1;

    if anc_type != MPEGS_ANCTYPE_HEADER_AND_FRAME && anc_type != MPEGS_ANCTYPE_FRAME {
        ret = 0;
    }

    if ret == 0 {
        update_mpeg_surround_decoder_status(
            d,
            MPEGS_INIT_ERROR_PAYLOAD,
            MPEGS_SYNC_LOST,
            MPEGS_STOP,
        );
    }

    ret
}

/// `isValidAncStartStop(pMpegSurroundDecoder, ancStartStop)`
fn is_valid_anc_start_stop(d: &mut CMpegSurroundDecoder, anc_start_stop: i32) -> i32 {
    let mut ret = 1;

    match anc_start_stop {
        MPEGS_START => {
            // Sequence start - start and continue - start not allowed
            if d.anc_start_stop_prev == MPEGS_START || d.anc_start_stop_prev == MPEGS_CONTINUE {
                ret = 0;
            }
        }
        MPEGS_STOP => {
            // MPS payload of the previous frame must be valid if current type is stop
            // Sequence startstop - stop and stop - stop not allowed
            // Sequence startstop - continue and stop - continue are allowed
            if d.anc_start_stop_prev == MPEGS_STOP || d.anc_start_stop_prev == MPEGS_START_STOP {
                ret = 0;
            }
        }
        _ => {
            // MPEGS_CONTINUE, MPEGS_START_STOP: no error detection possible for these states
        }
    }

    if ret == 0 {
        update_mpeg_surround_decoder_status(
            d,
            MPEGS_INIT_ERROR_PAYLOAD,
            MPEGS_SYNC_LOST,
            MPEGS_STOP,
        );
    } else {
        d.anc_start_stop_prev = anc_start_stop;
    }

    ret
}

/// The size of the buffer the payload of `mpsData` is parsed from (a power of two, see
/// [`mpeg_surround_decoder_parse`]).
const MPS_PARSE_BUFFER_SIZE: usize = 4096;

/// The bytes that follow `mpsData` in the C structure (`struct MpegSurroundDecoder`, the same
/// layout on both reference architectures): `mpsDataBits`, `spatialSpecificConfig[0]`, 4 bytes
/// of padding, the pointer `pSpatialDec` (not reproducible: 0 in the port),
/// `spatialSpecificConfigBackup`, `mpegSurroundDecoderLevel`, `mpegSurroundSscIsGlobalCfg`,
/// `mpegSurroundUseTimeInterface`. The bytes after these (padding, `bsFrames[0]`, ...) read as
/// 0.
fn struct_image_after_mps_data(d: &CMpegSurroundDecoder, out: &mut [u8]) {
    let mut v: Vec<u8> = Vec::with_capacity(512);
    v.extend_from_slice(&d.mps_data_bits.to_le_bytes());
    v.extend_from_slice(d.spatial_specific_config[0].as_bytes());
    v.extend_from_slice(&[0; 4]); // padding before pSpatialDec
    v.extend_from_slice(&[0; 8]); // pSpatialDec (not reproducible)
    v.extend_from_slice(d.spatial_specific_config_backup.as_bytes());
    v.push(d.mpeg_surround_decoder_level);
    v.push(d.mpeg_surround_ssc_is_global_cfg);
    v.push(d.mpeg_surround_use_time_interface);
    let n = v.len().min(out.len());
    out[..n].copy_from_slice(&v[..n]);
    out[n..].fill(0);
}

/// `mpegSurroundDecoder_Parse(pMpegSurroundDecoder, hBs, pMpsDataBits, coreCodec, sampleRate,
/// frameSize, fGlobalIndependencyFlag)`: parses MPEG Surround data with header (`ancType`,
/// `ancStartStop`) from the ancillary data of LD MPEG Surround.
///
/// # Reads beyond `mpsData`
///
/// The collected payload is parsed from `mpsData`; reads past the 2048 byte array continue into
/// the members that follow it in the C structure. The port reproduces this from the C layout:
/// it parses from a copy of `mpsData` followed by the image of these members
/// (`struct_image_after_mps_data`, taken when the frame parse starts). The bytes it cannot
/// reproduce read as 0, and positions beyond
/// `MPS_PARSE_BUFFER_SIZE` bytes wrap to the start of the copy (deviation).
///
/// Deviation: without a spatial decoder (no defined result in C) the port returns
/// `MPS_INVALID_HANDLE`.
pub fn mpeg_surround_decoder_parse(
    p_mpeg_surround_decoder: Option<&mut CMpegSurroundDecoder>,
    h_bs: &mut FdkBitStream,
    p_mps_data_bits: &mut i32,
    core_codec: AudioObjectType,
    sample_rate: i32,
    frame_size: i32,
    f_global_independency_flag: i32,
) -> i32 {
    let mut err = MPS_OK;

    let Some(d) = p_mpeg_surround_decoder else {
        return MPS_INVALID_HANDLE;
    };
    let mut mps_data_bits = *p_mps_data_bits;

    fdk_assert!(d.p_spatial_dec.is_some());
    if d.p_spatial_dec.is_none() {
        return MPS_INVALID_HANDLE;
    }

    let mps_bs_bits = fdk_get_valid_bits(h_bs) as i32;

    let parse = fi(d.bs_frame_parse);

    // The copy of mpsData that the payload is parsed from (MPEGS_STOP, MPEGS_CONTINUE).
    let mut parse_buf = vec![0u8; MPS_PARSE_BUFFER_SIZE];
    let mut mps_bs_data = FdkBitStream::default();
    // Which bitstream the frame is parsed from: false: hBs, true: mpsBsData.
    let mut use_mps_bs_data = false;

    'bail: {
        // Find operation mode of mpeg surround decoder:
        // - MPEGS_OPMODE_EMM:            Mode: Enhanced Matrix Mode (Blind)
        // - MPEGS_OPMODE_MPS_PAYLOAD:    Mode: Normal, Stereo or Binaural
        // - MPEGS_OPMODE_NO_MPS_PAYLOAD: Mode: No MpegSurround Payload
        {
            // Parse ancType and ancStartStop
            let anc_type = fdk_read_bits(h_bs, 2) as i32;
            let anc_start_stop = fdk_read_bits(h_bs, 2) as i32;
            mps_data_bits = mps_data_bits.wrapping_sub(4);

            // Set valid anc type flag, if ancType signals a payload with either header and
            // frame or frame
            if is_valid_anc_type(d, anc_type) == 0 {
                break 'bail;
            }
            // Set valid anc startstop flag, if transmitted sequence is not illegal
            if is_valid_anc_start_stop(d, anc_start_stop) == 0 {
                break 'bail;
            }

            match anc_start_stop {
                MPEGS_START => {
                    // Assuming that core coder frame size (AAC) is smaller than MPS coder frame
                    // size. Save audio data for next frame.
                    if mps_data_bits > (MPS_DATA_BUFFER_SIZE * 8) as i32 {
                        err = MPS_NOTOK;
                        break 'bail;
                    }
                    for i in 0..(mps_data_bits / 8).max(0) as usize {
                        d.mps_data[i] = fdk_read_bits(h_bs, 8) as u8;
                    }
                    d.mps_data_bits = mps_data_bits;
                }
                MPEGS_CONTINUE | MPEGS_STOP => {
                    // Assuming that core coder frame size (AAC) is smaller than MPS coder frame
                    // size. Save audio data for next frame.
                    if d.mps_data_bits.wrapping_add(mps_data_bits)
                        > (MPS_DATA_BUFFER_SIZE * 8) as i32
                    {
                        err = MPS_NOTOK;
                        break 'bail;
                    }
                    let base = d.mps_data_bits / 8;
                    for i in 0..(mps_data_bits / 8).max(0) {
                        let v = fdk_read_bits(h_bs, 8) as u8;
                        // Deviation: a byte outside the array (no defined result in C) is
                        // dropped.
                        if let Some(b) = d.mps_data.get_mut((base + i) as usize) {
                            *b = v;
                        }
                    }
                    d.mps_data_bits = d.mps_data_bits.wrapping_add(mps_data_bits);
                    parse_buf[..MPS_DATA_BUFFER_SIZE].copy_from_slice(&d.mps_data);
                    fdk_init_bit_stream(
                        &mut mps_bs_data,
                        &BitMem::from_slice(&parse_buf[..]),
                        MPS_PARSE_BUFFER_SIZE as u32,
                        d.mps_data_bits as u32,
                        BS_READER,
                    );
                    use_mps_bs_data = true;
                }
                MPEGS_START_STOP => {
                    d.mps_data_bits = mps_data_bits;
                    use_mps_bs_data = false;
                }
                _ => {
                    fdk_assert!(false);
                }
            }

            if anc_start_stop == MPEGS_STOP || anc_start_stop == MPEGS_START_STOP {
                let h_mps_bs_data: &mut FdkBitStream =
                    if use_mps_bs_data { &mut mps_bs_data } else { &mut *h_bs };
                let mut header_ok = true;
                match anc_type {
                    MPEGS_ANCTYPE_HEADER_AND_FRAME | MPEGS_ANCTYPE_FRAME => {
                        if anc_type == MPEGS_ANCTYPE_HEADER_AND_FRAME {
                            let spatial_specific_config_tmp = d.spatial_specific_config_backup;

                            // Parse spatial specific config
                            let mut bits_read = fdk_get_valid_bits(h_mps_bs_data) as i32;

                            err = spatial_dec_parse_specific_config_header(
                                h_mps_bs_data,
                                &mut d.spatial_specific_config_backup,
                                core_codec,
                                d.upmix_type,
                            );

                            bits_read -= fdk_get_valid_bits(h_mps_bs_data) as i32;
                            let mut parse_result =
                                if err == MPS_OK { bits_read } else { -bits_read };

                            if parse_result < 0 {
                                parse_result = -parse_result;
                                err = MPS_PARSE_ERROR;
                            } else if err == MPS_OK {
                                // Check SSC for consistency (e.g. bit errors could cause
                                // trouble)
                                err = ssc_check_in_band(
                                    &d.spatial_specific_config_backup,
                                    frame_size,
                                    sample_rate,
                                );
                            }
                            if err != MPS_OK {
                                d.spatial_specific_config_backup = spatial_specific_config_tmp;
                                header_ok = false;
                            } else {
                                d.mps_data_bits = d.mps_data_bits.wrapping_sub(parse_result);

                                // Initiate re-initialization, if header has changed
                                if fdk_spatial_dec_compare_spatial_specific_config_header(
                                    &d.spatial_specific_config_backup,
                                    &d.spatial_specific_config[parse],
                                ) == MPS_UNEQUAL_SSC
                                {
                                    d.init_flags[parse] |= MPEGS_INIT_CHANGE_HEADER;
                                    if let Some(sd) = d.p_spatial_dec.as_deref_mut() {
                                        spatial_dec_init_parser_context(sd);
                                    }
                                    // We found a valid in-band configuration. Therefore any
                                    // previous config is invalid now.
                                    d.mpeg_surround_ssc_is_global_cfg = 0;
                                }
                            }
                        }
                        // FDK_FALLTHROUGH to MPEGS_ANCTYPE_FRAME
                        if header_ok {
                            'frame: {
                                if d.init_flags[parse] & MPEGS_INIT_ERROR_PAYLOAD != 0 {
                                    err = MPS_PARSE_ERROR;
                                    break 'frame;
                                }

                                // First spatial specific config is parsed into
                                // spatialSpecificConfigBackup, second
                                // spatialSpecificConfigBackup is copied into
                                // spatialSpecificConfig[bsFrameDecode]
                                if d.init_flags[parse] != 0 {
                                    d.spatial_specific_config[parse] =
                                        d.spatial_specific_config_backup;
                                    d.f_on_sync[parse] = MPEGS_SYNC_FOUND;
                                }

                                if d.f_on_sync[parse] >= MPEGS_SYNC_FOUND {
                                    if err != MPS_OK {
                                        break 'frame;
                                    }

                                    if use_mps_bs_data {
                                        // The members after mpsData, as they are now (see the
                                        // function documentation).
                                        struct_image_after_mps_data(
                                            d,
                                            &mut parse_buf[MPS_DATA_BUFFER_SIZE..],
                                        );
                                    }

                                    let bits_avail = fdk_get_valid_bits(h_mps_bs_data) as i32;

                                    if bits_avail <= 0 {
                                        err = MPS_PARSE_ERROR;
                                    } else {
                                        let CMpegSurroundDecoder {
                                            p_spatial_dec,
                                            bs_frames,
                                            ll_state,
                                            spatial_specific_config,
                                            upmix_type,
                                            ..
                                        } = &mut *d;
                                        if let Some(sd) = p_spatial_dec.as_deref_mut() {
                                            err = spatial_dec_parse_frame_data(
                                                sd,
                                                &mut bs_frames[parse],
                                                ll_state,
                                                h_mps_bs_data,
                                                &spatial_specific_config[parse],
                                                *upmix_type,
                                                f_global_independency_flag,
                                            );
                                        }
                                        if err == MPS_OK {
                                            d.bs_frames[parse].new_bs_data = 1;
                                        }
                                    }

                                    let nbits = bits_avail
                                        .wrapping_sub(fdk_get_valid_bits(h_mps_bs_data) as i32);

                                    if nbits > bits_avail
                                        || nbits > d.mps_data_bits
                                        || (d.mps_data_bits > nbits.wrapping_add(7)
                                            && !is_lowdelay(core_codec))
                                    {
                                        d.bs_frames[parse].new_bs_data = 0;
                                        err = MPS_PARSE_ERROR;
                                        break 'frame;
                                    }
                                    d.mps_data_bits = d.mps_data_bits.wrapping_sub(nbits);
                                }
                            }
                        }
                    }
                    _ => {
                        // added to avoid compiler warning
                        err = MPS_NOTOK;
                    }
                } // switch (ancType)

                if err == MPS_OK {
                    d.anc_start_stop_prev = anc_start_stop;
                } else {
                    update_mpeg_surround_decoder_status(
                        d,
                        MPEGS_INIT_ERROR_PAYLOAD,
                        MPEGS_SYNC_LOST,
                        MPEGS_STOP,
                    );
                    d.mps_data_bits = 0;
                }
            } // (ancStartStop == MPEGS_STOP) || (ancStartStop == MPEGS_START_STOP)
        }
    }

    *p_mps_data_bits =
        p_mps_data_bits.wrapping_sub(mps_bs_bits.wrapping_sub(fdk_get_valid_bits(h_bs) as i32));

    err
}

/// `mpegSurroundDecoder_Apply(pMpegSurroundDecoder, input, pTimeData, timeDataSize,
/// timeDataFrameSize, nChannels, frameSize, sampleRate, coreCodec, channelType,
/// channelIndices, mapDescr, inDataHeadroom, outDataHeadroom)`: processes one downmix frame
/// (see the [module documentation](self) for `qd`, `input` and `p_time_data`).
///
/// Deviation: without a spatial decoder (no defined result in C) the port returns
/// `MPS_INVALID_HANDLE`.
pub fn mpeg_surround_decoder_apply(
    p_mpeg_surround_decoder: Option<&mut CMpegSurroundDecoder>,
    qd: &mut FdkQmfDomain,
    input: &mut [PcmMps],
    p_time_data: &mut [PcmMps],
    time_data_size: i32,
    time_data_frame_size: i32,
    n_channels: &mut i32,
    frame_size: &mut i32,
    sample_rate: i32,
    core_codec: AudioObjectType,
    channel_type: Option<&mut [AudioChannelType]>,
    channel_indices: Option<&mut [u8]>,
    map_descr: Option<&FdkChannelMapDescr>,
    in_data_headroom: i32,
    out_data_headroom: &mut i32,
) -> i32 {
    let mut err = MPS_OK;
    let init_control_flags: u32;
    let mut control_flags: u32 = 0;

    let Some(d) = p_mpeg_surround_decoder else {
        return MPS_INVALID_HANDLE;
    };

    fdk_assert!(d.p_spatial_dec.is_some());
    if d.p_spatial_dec.is_none() {
        return MPS_INVALID_HANDLE;
    }

    if fdk_ch_map_descr_is_valid(map_descr) == 0 {
        return MPS_INVALID_HANDLE;
    }
    let Some(map_descr) = map_descr else {
        return MPS_INVALID_HANDLE;
    };

    if *n_channels <= 0 || *n_channels > 2 {
        return MPS_NOTOK;
    }

    let dec = fi(d.bs_frame_decode);
    let parse = fi(d.bs_frame_parse);

    if let Some(sd) = d.p_spatial_dec.as_deref_mut() {
        sd.sac_in_data_headroom = in_data_headroom;
        // pConfigCurrent = &spatialSpecificConfig[bsFrameDecode]
        sd.p_config_current = true;
    }
    *out_data_headroom = 8;

    let new_data = d.bs_frames[parse].new_bs_data;

    'bail: {
        match mpeg_surround_operation_mode(d, 1000) {
            MPEGS_OPMODE_MPS_PAYLOAD => {
                if d.init_flags[dec] != 0 {
                    err = init_mpeg_surround_decoder(d, qd);
                }

                if err == MPS_OK {
                    if d.f_on_sync[dec] != MPEGS_SYNC_COMPLETE
                        && d.bs_frames[dec].bs_independency_flag == 1
                    {
                        // We got a valid header and independently decodeable frame data.
                        //  -> Go to the next sync level and start processing.
                        d.f_on_sync[dec] = MPEGS_SYNC_COMPLETE;
                    }
                } else {
                    // We got a valid config header but found an error while parsing the
                    // bitstream. Wait for the next independent frame and apply error
                    // conealment in the meantime.
                    d.f_on_sync[dec] = MPEGS_SYNC_FOUND;
                    control_flags |= MPEGS_CONCEAL;
                    err = MPS_OK;
                }
                // Concealment:
                // - Bitstream is available, no sync found during bitstream processing
                // - Bitstream is available, sync lost due to corrupted bitstream
                // - Bitstream is available, sync found but no independent frame
                if d.f_on_sync[dec] != MPEGS_SYNC_COMPLETE {
                    control_flags |= MPEGS_CONCEAL;
                }
            }
            MPEGS_OPMODE_NO_MPS_PAYLOAD => {
                // Concealment: No bitstream is available
                control_flags |= MPEGS_CONCEAL;
            }
            _ => {
                err = MPS_NOTOK;
            }
        }

        if err != MPS_OK {
            break 'bail;
        }

        // Force BypassMode if choosen by user
        if d.mpeg_surround_user_params.bypass_mode != 0 {
            control_flags |= MPEGS_BYPASSMODE;
        }

        if d.init_flags[dec] != 0 {
            let mut start_with_dflt_cfg = 0;
            // Init with a default configuration if we came here and are still not initialized.
            if d.init_flags[dec] & MPEGS_INIT_ENFORCE_REINIT != 0 {
                // Get default spatial specific config
                if fdk_spatial_dec_init_default_spatial_specific_config(
                    &mut d.spatial_specific_config_backup,
                    core_codec,
                    *n_channels,
                    sample_rate,
                    frame_size.wrapping_div(mpeg_surround_decoder_get_nr_of_qmf_bands(
                        None,
                        sample_rate as u32,
                    )),
                    d.mpeg_surround_decoder_level as i32,
                    d.mpeg_surround_user_params.blind_enable as i32,
                ) != 0
                {
                    err = MPS_NOTOK;
                    break 'bail;
                }

                // Initiate re-initialization, if header has changed
                if fdk_spatial_dec_compare_spatial_specific_config_header(
                    &d.spatial_specific_config_backup,
                    &d.spatial_specific_config[dec],
                ) == MPS_UNEQUAL_SSC
                {
                    d.init_flags[dec] |= MPEGS_INIT_CHANGE_HEADER;
                    if let Some(sd) = d.p_spatial_dec.as_deref_mut() {
                        spatial_dec_init_parser_context(sd);
                    }
                }

                start_with_dflt_cfg = 1;
            }

            // First spatial specific config is parsed into spatialSpecificConfigBackup, second
            // spatialSpecificConfigBackup is copied into spatialSpecificConfig
            err = init_mpeg_surround_decoder(d, qd);

            if start_with_dflt_cfg != 0 {
                // initialized with default config, but no sync found
                // maybe use updateMpegSurroundDecoderStatus later on
                d.f_on_sync[dec] = MPEGS_SYNC_LOST;
            }

            // Since we do not have state MPEGS_SYNC_COMPLETE apply concealment
            control_flags |= MPEGS_CONCEAL;

            if err != MPS_OK {
                break 'bail;
            }
        }

        // Process MPEG Surround Audio
        init_control_flags = control_flags;

        // Check that provided output buffer is large enough.
        if qd.global_conf.n_bands_analysis == 0 {
            err = MPS_UNSUPPORTED_FORMAT;
            break 'bail;
        }
        let num_output_channels_at =
            d.p_spatial_dec.as_deref().map_or(0, |sd| sd.num_output_channels_at);
        let stereo_config_index = d.p_spatial_dec.as_deref().map_or(0, |sd| sd.stereo_config_index);
        let n_bands_synthesis = qd.global_conf.n_bands_synthesis as i32;
        let n_bands_analysis = qd.global_conf.n_bands_analysis as i32;
        let time_data_required_size = (time_data_frame_size
            .wrapping_mul(num_output_channels_at)
            .wrapping_mul(n_bands_synthesis))
            / n_bands_analysis;
        if time_data_size < time_data_required_size {
            err = MPS_OUTPUT_BUFFER_TOO_SMALL;
            break 'bail;
        }

        let in_data: PcmMpsInput<'_>;
        let input_ro: &[PcmMps];
        if d.spatial_specific_config[dec].syntax_flags & SACDEC_SYNTAX_USAC != 0
            && stereo_config_index > 1
        {
            fdk_assert!(time_data_required_size >= time_data_frame_size * *n_channels);
            // Place samples comprising QMF time slots spaced at QMF output Band raster to allow
            // slot wise processing
            let time_data_frame_size_out =
                (time_data_frame_size.wrapping_mul(n_bands_synthesis)) / n_bands_analysis;
            in_data = PcmMpsInput::Output(
                (time_data_frame_size_out - time_data_frame_size).max(0) as usize
            );
            for i in (0..*n_channels).rev() {
                // Deviation: parts outside of the buffer are not moved or cleared.
                let dst =
                    (i + 1) as i64 * time_data_frame_size_out as i64 - time_data_frame_size as i64;
                let src = time_data_frame_size as i64 * i as i64;
                move_within(p_time_data, src, dst, time_data_frame_size as i64);
                clear_range(
                    p_time_data,
                    i as i64 * time_data_frame_size_out as i64,
                    (time_data_frame_size_out - time_data_frame_size) as i64,
                );
            }
            input_ro = &[];
        } else {
            if d.mpeg_surround_use_time_interface != 0 {
                let n = (*n_channels as i64 * *frame_size as i64).max(0) as usize;
                // Deviation: as above for buffers shorter than nChannels * frameSize.
                let n = n.min(input.len()).min(p_time_data.len());
                input[..n].copy_from_slice(&p_time_data[..n]);
                input_ro = &*input;
                in_data = PcmMpsInput::Buffer(input_ro);
            } else {
                input_ro = &[];
                in_data = PcmMpsInput::None;
            }
        }
        let _ = input_ro;

        // Process MPEG Surround Audio
        {
            let use_time = d.mpeg_surround_use_time_interface != 0;
            let CMpegSurroundDecoder {
                p_spatial_dec,
                spatial_specific_config,
                bs_frames,
                ll_state,
                ..
            } = &mut *d;
            if let Some(sd) = p_spatial_dec.as_deref_mut() {
                err = spatial_dec_apply_frame(
                    sd,
                    qd,
                    &mut spatial_specific_config[dec],
                    &mut bs_frames[dec],
                    ll_state,
                    if use_time { INPUTMODE_TIME } else { INPUTMODE_QMF_SBR },
                    in_data,
                    p_time_data,
                    *frame_size as u32,
                    &mut control_flags,
                    *n_channels,
                    map_descr,
                );
            }
        }
        *n_channels = num_output_channels_at_now(d);

        if err != MPS_OK {
            // A fatal error occured. Go back to start and try again:
            update_mpeg_surround_decoder_status(
                d,
                MPEGS_INIT_ENFORCE_REINIT,
                MPEGS_SYNC_LOST,
                MPEGS_STOP,
            );
            *frame_size = 0; // Declare that framework can not use the data in pTimeOut.
        } else {
            let err_int = d.p_spatial_dec.as_deref().map_or(MPS_OK, |sd| sd.err_int);
            if (control_flags & MPEGS_CONCEAL != 0 && init_control_flags & MPEGS_CONCEAL == 0)
                || err_int != MPS_OK
            {
                // Account for errors that occured in SpatialDecApplyFrame():
                update_mpeg_surround_decoder_status(
                    d,
                    MPEGS_INIT_ERROR_PAYLOAD,
                    MPEGS_SYNC_LOST,
                    MPEGS_STOP,
                );
            }
        }

        if err == MPS_OK
            && control_flags & MPEGS_BYPASSMODE == 0
            && d.upmix_type != UPMIX_TYPE_BYPASS
        {
            spatial_dec_channel_properties(
                d.p_spatial_dec.as_deref(),
                channel_type,
                channel_indices,
                Some(map_descr),
            );
        }
    }

    if new_data != 0 {
        // numParameterSetsPrev shall only be read in the decode process, because of that we
        // can update this state variable here
        let n = d.bs_frames[dec].num_parameter_sets;
        if let Some(sd) = d.p_spatial_dec.as_deref_mut() {
            sd.num_parameter_sets_prev = n;
        }
    }

    err
}

/// `pSpatialDec->numOutputChannelsAT`
fn num_output_channels_at_now(d: &CMpegSurroundDecoder) -> i32 {
    d.p_spatial_dec.as_deref().map_or(0, |sd| sd.num_output_channels_at)
}

/// `FDKmemmove(buf + dst, buf + src, n * sizeof(PCM_MPS))` within the buffer (only the part
/// of both ranges inside the buffer).
fn move_within(buf: &mut [PcmMps], src: i64, dst: i64, n: i64) {
    let len = buf.len() as i64;
    if n <= 0 || src < 0 || dst < 0 {
        return;
    }
    let n = n.min(len - src).min(len - dst);
    if n > 0 {
        buf.copy_within(src as usize..(src + n) as usize, dst as usize);
    }
}

/// `FDKmemclear(buf + pos, n * sizeof(PCM_MPS))` (only the part inside the buffer).
fn clear_range(buf: &mut [PcmMps], pos: i64, n: i64) {
    let len = buf.len() as i64;
    if n <= 0 || pos < 0 || pos >= len {
        return;
    }
    let e = (pos + n).min(len);
    buf[pos as usize..e as usize].fill(0);
}

/// `mpegSurroundDecoder_FreeMem(pMpegSurroundDecoder)`: frees the config dependent memory (the
/// spatial decoder).
pub fn mpeg_surround_decoder_free_mem(
    p_mpeg_surround_decoder: Option<&mut CMpegSurroundDecoder>,
) -> SacdecError {
    if let Some(d) = p_mpeg_surround_decoder {
        fdk_spatial_dec_close(d.p_spatial_dec.take());
    }
    MPS_OK
}

/// `mpegSurroundDecoder_Close(pMpegSurroundDecoder)`: deallocates a decoder instance.
pub fn mpeg_surround_decoder_close(p_mpeg_surround_decoder: Option<Box<CMpegSurroundDecoder>>) {
    if let Some(mut d) = p_mpeg_surround_decoder {
        fdk_spatial_dec_close(d.p_spatial_dec.take());
        for i in 0..1 {
            spatial_dec_close_bs_frame(Some(&mut d.bs_frames[i]));
        }
    }
}

/// `SACDEC_VL0`
const SACDEC_VL0: i32 = 2;
/// `SACDEC_VL1`
const SACDEC_VL1: i32 = 1;
/// `SACDEC_VL2`
const SACDEC_VL2: i32 = 0;

/// `mpegSurroundDecoder_GetLibInfo(libInfo)`: fills the next free entry of the library info
/// list. Returns 0 on success, -1 on failure. The build date and time are empty strings
/// (a deviation).
pub fn mpeg_surround_decoder_get_lib_info(info: Option<&mut [LibInfo]>) -> i32 {
    let Some(info) = info else {
        return -1;
    };

    // search for next free tab
    let mut i = 0usize;
    while i < FDK_MODULE_LAST as usize {
        match info.get(i) {
            Some(e) if e.module_id == FDK_NONE => break,
            Some(_) => {}
            // Deviation: guarded at the end of the caller's array.
            None => return -1,
        }
        i += 1;
    }
    if i == FDK_MODULE_LAST as usize {
        return -1;
    }
    let info = &mut info[i];

    info.module_id = FDK_MPSDEC;
    info.build_date = "";
    info.build_time = "";
    info.title = "MPEG Surround Decoder";
    info.version = lib_version(SACDEC_VL0, SACDEC_VL1, SACDEC_VL2);
    lib_version_string(info);
    info.flags = CAPF_MPS_LD | CAPF_MPS_USAC | CAPF_MPS_HQ | CAPF_MPS_1CH_IN | CAPF_MPS_2CH_OUT; // end flags

    0
}

/// `mpegSurroundDecoder_SetParam(pMpegSurroundDecoder, param, value)`: sets one decoder
/// parameter.
pub fn mpeg_surround_decoder_set_param(
    p_mpeg_surround_decoder: Option<&mut CMpegSurroundDecoder>,
    param: SacdecParam,
    value: i32,
) -> SacdecError {
    let mut err = MPS_OK;

    // check decoder handle
    if p_mpeg_surround_decoder.is_none() {
        err = MPS_INVALID_HANDLE;
        // check the parameter values before exiting.
    }
    let mut d = p_mpeg_surround_decoder;

    // apply param value
    match param {
        SACDEC_OUTPUT_MODE => {
            match value {
                SACDEC_OUT_MODE_NORMAL | SACDEC_OUT_MODE_STEREO => {}
                _ => err = MPS_INVALID_PARAMETER,
            }
            if err == MPS_OK {
                if let Some(d) = d.as_deref_mut() {
                    if d.mpeg_surround_user_params.output_mode != value as u8 {
                        d.mpeg_surround_user_params.output_mode = value as u8;
                        let dec = fi(d.bs_frame_decode);
                        d.init_flags[dec] |= MPEGS_INIT_CHANGE_OUTPUT_MODE;
                    }
                }
            }
        }
        SACDEC_INTERFACE => {
            if value < 0 || value > 1 {
                err = MPS_INVALID_PARAMETER;
            }
            if err != MPS_OK {
                return err;
            }
            if let Some(d) = d.as_deref_mut() {
                if d.mpeg_surround_use_time_interface != value as u8 {
                    d.mpeg_surround_use_time_interface = value as u8;
                    let dec = fi(d.bs_frame_decode);
                    d.init_flags[dec] |= MPEGS_INIT_CHANGE_TIME_FREQ_INTERFACE;
                }
            }
        }
        SACDEC_BS_INTERRUPTION => {
            if err == MPS_OK && value != 0 {
                if let Some(d) = d.as_deref_mut() {
                    update_mpeg_surround_decoder_status(
                        d,
                        MPEGS_INIT_BS_INTERRUPTION,
                        MPEGS_SYNC_LOST,
                        MPEGS_STOP,
                    );
                }
            }
        }
        SACDEC_CLEAR_HISTORY => {
            if err == MPS_OK && value != 0 {
                // Just reset the states and go on.
                if let Some(d) = d.as_deref_mut() {
                    update_mpeg_surround_decoder_status(
                        d,
                        MPEGS_INIT_CLEAR_HISTORY,
                        MPEGS_SYNC_LOST,
                        MPEGS_STOP,
                    );
                }
            }
        }
        SACDEC_CONCEAL_NUM_KEEP_FRAMES
        | SACDEC_CONCEAL_FADE_OUT_SLOPE_LENGTH
        | SACDEC_CONCEAL_FADE_IN_SLOPE_LENGTH
        | SACDEC_CONCEAL_NUM_RELEASE_FRAMES => {
            if value < 0 {
                // Check valid value range
                err = MPS_INVALID_PARAMETER;
            }
            if err != MPS_OK {
                return err;
            }
            if let Some(d) = d.as_mut() {
                let p = &mut d.mpeg_surround_user_params;
                let field = match param {
                    SACDEC_CONCEAL_NUM_KEEP_FRAMES => &mut p.conceal_num_keep_frames,
                    SACDEC_CONCEAL_FADE_OUT_SLOPE_LENGTH => &mut p.conceal_fade_out_slope_length,
                    SACDEC_CONCEAL_FADE_IN_SLOPE_LENGTH => &mut p.conceal_fade_in_slope_length,
                    _ => &mut p.conceal_num_release_frames,
                };
                if *field != value as u32 {
                    *field = value as u32;
                    let dec = fi(d.bs_frame_decode);
                    d.init_flags[dec] |= MPEGS_INIT_CHANGE_CONCEAL_PARAMS;
                }
            }
        }
        _ => {
            err = MPS_INVALID_PARAMETER;
        }
    } // switch(param)

    err
}

/// `mpegSurroundDecoder_IsPseudoLR(pMpegSurroundDecoder, bsPseudoLr)`: whether the USAC pseudo
/// LR feature is active.
pub fn mpeg_surround_decoder_is_pseudo_lr(
    p_mpeg_surround_decoder: Option<&CMpegSurroundDecoder>,
    bs_pseudo_lr: &mut i32,
) -> SacdecError {
    if let Some(d) = p_mpeg_surround_decoder {
        let ssc_decode = &d.spatial_specific_config[fi(d.bs_frame_decode)];
        *bs_pseudo_lr = ssc_decode.bs_pseudo_lr as i32;
        MPS_OK
    } else {
        MPS_INVALID_HANDLE
    }
}

/// `mpegSurroundDecoder_GetDelay(self)`: the additional signal delay caused by the module.
pub fn mpeg_surround_decoder_get_delay(self_: Option<&CMpegSurroundDecoder>) -> u32 {
    let mut output_delay: i32 = 0;

    if let Some(s) = self_ {
        let ssc_decode = &s.spatial_specific_config[fi(s.bs_frame_decode)];
        let core_codec = ssc_decode.core_codec;

        // See chapter 4.5 (delay and synchronization) of ISO/IEC FDIS 23003-1 and chapter 5.4.3
        // of ISO/IEC FDIS 23003-2 for details on the following figures.

        if core_codec > AOT_NULL_OBJECT {
            if is_lowdelay(core_codec) {
                // All low delay variants (ER-AAC-(E)LD):
                output_delay += 256;
            } else if !is_usac(core_codec) {
                // By the method of elimination this is the GA (AAC-LC, HE-AAC, ...) branch:
                output_delay += 320 + 257; // cos to exp delay + QMF synthesis
                if s.mpeg_surround_use_time_interface != 0 {
                    output_delay += 320 + 384; // QMF and hybrid analysis
                }
            }
        }
    }

    output_delay as u32
}
