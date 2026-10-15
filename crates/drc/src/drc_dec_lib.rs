// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libDRCdec/include/FDK_drcDecLib.h, libDRCdec/src/FDK_drcDecLib.cpp.

//! Port of `FDK_drcDecLib.h`: the MPEG-D DRC decoder according to ISO/IEC 23003-4 (MPEG-D DRC),
//! including ISO/IEC 23003-4/AMD1 (Amendment 1).
//!
//! Calling sequence: `FDK_drcDec_Read...()`, `FDK_drcDec_SetChannelGains()`,
//! `FDK_drcDec_Preprocess()`, `FDK_drcDec_Process...()`.
//!
//! The handles are `Option<&mut DrcDecoder>` where the C functions check for `NULL`. Where the
//! reference has no defined result for a `NULL` handle, the port returns as noted at the
//! function.

use fdk_aac_rs_fdk::audio::{
    lib_version, lib_version_string, LibInfo, FDK_MODULE_LAST, FDK_NONE, FDK_UNIDRCDEC,
};
use fdk_aac_rs_fdk::bitstream::FdkBitStream;
use fdk_aac_rs_fdk::common_fix::{f_mult_div2, FixpDbl};

use crate::drc_dec_gain_decoder::*;
use crate::drc_dec_reader::*;
use crate::drc_dec_selection_process::*;
use crate::drc_dec_tools::*;
use crate::drc_dec_types::*;
use crate::drc_decoder::*;

/// `DRC_DEC_LOUDNESS_NOT_PRESENT`
pub const DRC_DEC_LOUDNESS_NOT_PRESENT: i32 = 0x7FFF_FFFE;

/// `DRC_DEC_FUNCTIONAL_RANGE`
pub type DrcDecFunctionalRange = i32;
/// `DRC_DEC_SELECTION`: DRC decoder instance for DRC set selection only
pub const DRC_DEC_SELECTION: DrcDecFunctionalRange = 0x1;
/// `DRC_DEC_GAIN`: DRC decoder instance for applying DRC only
pub const DRC_DEC_GAIN: DrcDecFunctionalRange = 0x2;
/// `DRC_DEC_ALL`: DRC decoder with full functionality
pub const DRC_DEC_ALL: DrcDecFunctionalRange = 0x3;

/// `DRC_DEC_USERPARAM`
pub type DrcDecUserParam = i32;
/* get and set userparams */
/// `DRC_DEC_BOOST`
pub const DRC_DEC_BOOST: DrcDecUserParam = 0;
/// `DRC_DEC_COMPRESS`
pub const DRC_DEC_COMPRESS: DrcDecUserParam = 1;
/* set only userparams */
/// `DRC_DEC_LOUDNESS_NORMALIZATION_ON`
pub const DRC_DEC_LOUDNESS_NORMALIZATION_ON: DrcDecUserParam = 2;
/// `DRC_DEC_TARGET_LOUDNESS`: target loudness in dB, with exponent e = 7
pub const DRC_DEC_TARGET_LOUDNESS: DrcDecUserParam = 3;
/// `DRC_DEC_EFFECT_TYPE`
pub const DRC_DEC_EFFECT_TYPE: DrcDecUserParam = 4;
/// `DRC_DEC_EFFECT_TYPE_FALLBACK_CODE`
pub const DRC_DEC_EFFECT_TYPE_FALLBACK_CODE: DrcDecUserParam = 5;
/// `DRC_DEC_LOUDNESS_MEASUREMENT_METHOD`
pub const DRC_DEC_LOUDNESS_MEASUREMENT_METHOD: DrcDecUserParam = 6;
/// `DRC_DEC_ALBUM_MODE`
pub const DRC_DEC_ALBUM_MODE: DrcDecUserParam = 7;
/* set only system (not user) parameters */
/// `DRC_DEC_DOWNMIX_ID`
pub const DRC_DEC_DOWNMIX_ID: DrcDecUserParam = 8;
/// `DRC_DEC_TARGET_CHANNEL_COUNT_REQUESTED`: number of output channels notified to
/// FDK_drcDecLib for choosing an appropriate downmixInstruction
pub const DRC_DEC_TARGET_CHANNEL_COUNT_REQUESTED: DrcDecUserParam = 9;
/// `DRC_DEC_BASE_CHANNEL_COUNT`
pub const DRC_DEC_BASE_CHANNEL_COUNT: DrcDecUserParam = 10;
/// `DRC_DEC_FRAME_SIZE`
pub const DRC_DEC_FRAME_SIZE: DrcDecUserParam = 11;
/// `DRC_DEC_SAMPLE_RATE`
pub const DRC_DEC_SAMPLE_RATE: DrcDecUserParam = 12;
/* get only system parameters */
/// `DRC_DEC_IS_MULTIBAND_DRC_1`
pub const DRC_DEC_IS_MULTIBAND_DRC_1: DrcDecUserParam = 13;
/// `DRC_DEC_IS_MULTIBAND_DRC_2`
pub const DRC_DEC_IS_MULTIBAND_DRC_2: DrcDecUserParam = 14;
/// `DRC_DEC_IS_ACTIVE`: MPEG-D DRC payload is present and at least one of Dynamic Range Control
/// (DRC) or Loudness Normalization (LN) is activated
pub const DRC_DEC_IS_ACTIVE: DrcDecUserParam = 15;
/// `DRC_DEC_TARGET_CHANNEL_COUNT_SELECTED`: number of output channels if appropriate
/// downmixInstruction exists
pub const DRC_DEC_TARGET_CHANNEL_COUNT_SELECTED: DrcDecUserParam = 16;
/// `DRC_DEC_OUTPUT_LOUDNESS`: output loudness in dB, with exponent e = 7, or
/// DRC_DEC_LOUDNESS_NOT_PRESENT if no loudness is contained in the bitstream
pub const DRC_DEC_OUTPUT_LOUDNESS: DrcDecUserParam = 17;

/// `DRC_DEC_ERROR`
pub type DrcDecError = i32;
/// `DRC_DEC_OK`
pub const DRC_DEC_OK: DrcDecError = 0;
/// `DRC_DEC_NOT_OK`
pub const DRC_DEC_NOT_OK: DrcDecError = -10000;
/// `DRC_DEC_OUT_OF_MEMORY`
pub const DRC_DEC_OUT_OF_MEMORY: DrcDecError = -9999;
/// `DRC_DEC_NOT_OPENED`
pub const DRC_DEC_NOT_OPENED: DrcDecError = -9998;
/// `DRC_DEC_NOT_READY`
pub const DRC_DEC_NOT_READY: DrcDecError = -9997;
/// `DRC_DEC_PARAM_OUT_OF_RANGE`
pub const DRC_DEC_PARAM_OUT_OF_RANGE: DrcDecError = -9996;
/// `DRC_DEC_INVALID_PARAM`
pub const DRC_DEC_INVALID_PARAM: DrcDecError = -9995;
/// `DRC_DEC_UNSUPPORTED_FUNCTION`
pub const DRC_DEC_UNSUPPORTED_FUNCTION: DrcDecError = -9994;

/// `DRC_DEC_CODEC_MODE`
pub type DrcDecCodecMode = i32;
/// `DRC_DEC_TEST_TIME_DOMAIN`
pub const DRC_DEC_TEST_TIME_DOMAIN: DrcDecCodecMode = -100;
/// `DRC_DEC_TEST_QMF_DOMAIN`
pub const DRC_DEC_TEST_QMF_DOMAIN: DrcDecCodecMode = -99;
/// `DRC_DEC_TEST_STFT_DOMAIN`
pub const DRC_DEC_TEST_STFT_DOMAIN: DrcDecCodecMode = -98;
/// `DRC_DEC_CODEC_MODE_UNDEFINED`
pub const DRC_DEC_CODEC_MODE_UNDEFINED: DrcDecCodecMode = -1;
/// `DRC_DEC_MPEG_4_AAC`
pub const DRC_DEC_MPEG_4_AAC: DrcDecCodecMode = 0;
/// `DRC_DEC_MPEG_D_USAC`
pub const DRC_DEC_MPEG_D_USAC: DrcDecCodecMode = 1;
/// `DRC_DEC_MPEG_H_3DA`
pub const DRC_DEC_MPEG_H_3DA: DrcDecCodecMode = 2;

/// `DRC_DEC_LOCATION`: apply only DRC sets dedicated to processing location.
/// DRC1: before downmix, DRC2: before or after downmix (AMD1: only after downmix), DRC3: after
/// downmix
pub type DrcDecLocation = i32;
/// `DRC_DEC_DRC1`
pub const DRC_DEC_DRC1: DrcDecLocation = 0;
/// `DRC_DEC_DRC1_DRC2`
pub const DRC_DEC_DRC1_DRC2: DrcDecLocation = 1;
/// `DRC_DEC_DRC2`
pub const DRC_DEC_DRC2: DrcDecLocation = 2;
/// `DRC_DEC_DRC3`
pub const DRC_DEC_DRC3: DrcDecLocation = 3;
/// `DRC_DEC_DRC2_DRC3`
pub const DRC_DEC_DRC2_DRC3: DrcDecLocation = 4;

/* Decoder library info */
/// `DRCDEC_LIB_VL0`
pub const DRCDEC_LIB_VL0: i32 = 2;
/// `DRCDEC_LIB_VL1`
pub const DRCDEC_LIB_VL1: i32 = 1;
/// `DRCDEC_LIB_VL2`
pub const DRCDEC_LIB_VL2: i32 = 0;
/// `DRCDEC_LIB_TITLE`
pub const DRCDEC_LIB_TITLE: &str = "MPEG-D DRC Decoder Lib";
/// `DRCDEC_LIB_BUILD_DATE` (`__DATE__` in the reference; empty)
pub const DRCDEC_LIB_BUILD_DATE: &str = "";
/// `DRCDEC_LIB_BUILD_TIME` (`__TIME__` in the reference; empty)
pub const DRCDEC_LIB_BUILD_TIME: &str = "";

/// `DRC_DEC_STATUS`
pub type DrcDecStatus = i32;
/// `DRC_DEC_NOT_INITIALIZED`
pub const DRC_DEC_NOT_INITIALIZED: DrcDecStatus = 0;
/// `DRC_DEC_INITIALIZED`
pub const DRC_DEC_INITIALIZED: DrcDecStatus = 1;
/// `DRC_DEC_NEW_GAIN_PAYLOAD`
pub const DRC_DEC_NEW_GAIN_PAYLOAD: DrcDecStatus = 2;
/// `DRC_DEC_INTERPOLATION_PREPARED`
pub const DRC_DEC_INTERPOLATION_PREPARED: DrcDecStatus = 3;

/// `struct s_drc_decoder` (`DRC_DECODER`)
#[derive(Clone, Debug)]
pub struct DrcDecoder {
    pub codec_mode: DrcDecCodecMode,
    pub functional_range: DrcDecFunctionalRange,
    pub status: DrcDecStatus,

    /* handles of submodules */
    pub h_gain_dec: HandleDrcGainDecoder,
    pub h_selection_proc: HandleDrcSelectionProcess,
    pub sel_proc_input_diff: i32,

    /* data structs */
    pub uni_drc_config: Box<UniDrcConfig>,
    pub loudness_info_set: Box<LoudnessInfoSet>,
    pub uni_drc_gain: Box<UniDrcGain>,

    pub sel_proc_output: SelProcOutput,
}

/// `HANDLE_DRC_DECODER`
pub type HandleDrcDecoder = Option<Box<DrcDecoder>>;

/// `HANDLE_SEL_PROC_OUTPUT`
pub type HandleSelProcOutput<'a> = Option<&'a SelProcOutput>;

impl DrcDecoder {
    /// The cleared structure (`FDKcalloc`).
    fn zeroed() -> Self {
        DrcDecoder {
            codec_mode: 0,
            functional_range: 0,
            status: 0,
            h_gain_dec: None,
            h_selection_proc: None,
            sel_proc_input_diff: 0,
            uni_drc_config: Box::new(UniDrcConfig::ZERO),
            loudness_info_set: Box::new(LoudnessInfoSet::ZERO),
            uni_drc_gain: Box::new(UniDrcGain::ZERO),
            sel_proc_output: SelProcOutput::ZERO,
        }
    }
}

/// `_getGainStatus(hUniDrcGain)`
fn get_gain_status(h_uni_drc_gain: &UniDrcGain) -> i32 {
    h_uni_drc_gain.status as i32
}

/// `isResetNeeded(hDrcDec, oldSelProcOutput)`
fn is_reset_needed(h_drc_dec: &DrcDecoder, old_sel_proc_output: &SelProcOutput) -> i32 {
    let mut reset_needed = 0;
    let out = &h_drc_dec.sel_proc_output;

    if out.num_selected_drc_sets != old_sel_proc_output.num_selected_drc_sets {
        reset_needed = 1;
    } else {
        // numSelectedDrcSets is at most 3 (selection process).
        for i in 0..(out.num_selected_drc_sets as usize).min(MAX_ACTIVE_DRCS) {
            if out.selected_drc_set_ids[i] != old_sel_proc_output.selected_drc_set_ids[i] {
                reset_needed = 1;
            }
            if out.selected_downmix_ids[i] != old_sel_proc_output.selected_downmix_ids[i] {
                reset_needed = 1;
            }
        }
    }

    if out.boost != old_sel_proc_output.boost {
        reset_needed = 1;
    }
    if out.compress != old_sel_proc_output.compress {
        reset_needed = 1;
    }

    /* Note: Changes in downmix matrix are not caught, as they don't affect the DRC gain
     * decoder */

    reset_needed
}

/// `startSelectionProcess(hDrcDec)`
fn start_selection_process(h_drc_dec: &mut DrcDecoder) {
    let mut uni_drc_config_has_changed = 0;
    let old_sel_proc_output = h_drc_dec.sel_proc_output;

    if h_drc_dec.status == 0 {
        return;
    }

    if h_drc_dec.functional_range & DRC_DEC_SELECTION != 0 {
        uni_drc_config_has_changed = h_drc_dec.uni_drc_config.diff as i32;
        if h_drc_dec.uni_drc_config.diff != 0
            || h_drc_dec.loudness_info_set.diff != 0
            || h_drc_dec.sel_proc_input_diff != 0
        {
            /* in case of an error, signal that selection process was not successful */
            h_drc_dec.sel_proc_output.num_selected_drc_sets = 0;

            drc_dec_selection_process_process(
                h_drc_dec.h_selection_proc.as_deref_mut(),
                &mut h_drc_dec.uni_drc_config,
                &h_drc_dec.loudness_info_set,
                &mut h_drc_dec.sel_proc_output,
            );

            h_drc_dec.sel_proc_input_diff = 0;
            h_drc_dec.uni_drc_config.diff = 0;
            h_drc_dec.loudness_info_set.diff = 0;
        }
    }

    if h_drc_dec.functional_range & DRC_DEC_GAIN != 0 {
        if is_reset_needed(h_drc_dec, &old_sel_proc_output) != 0 || uni_drc_config_has_changed != 0
        {
            if let Some(h_gain_dec) = h_drc_dec.h_gain_dec.as_deref_mut() {
                let out = &h_drc_dec.sel_proc_output;
                drc_dec_gain_decoder_config(
                    h_gain_dec,
                    &h_drc_dec.uni_drc_config,
                    out.num_selected_drc_sets,
                    &out.selected_drc_set_ids,
                    &out.selected_downmix_ids,
                );
            }
        }
    }
}

/// `FDK_drcDec_Open(phDrcDec, functionalRange)`
pub fn fdk_drc_dec_open(
    ph_drc_dec: &mut HandleDrcDecoder,
    functional_range: DrcDecFunctionalRange,
) -> DrcDecError {
    *ph_drc_dec = Some(Box::new(DrcDecoder::zeroed()));
    let Some(h_drc_dec) = ph_drc_dec.as_deref_mut() else {
        return DRC_DEC_OUT_OF_MEMORY;
    };

    h_drc_dec.functional_range = functional_range;

    h_drc_dec.status = DRC_DEC_NOT_INITIALIZED;
    h_drc_dec.codec_mode = DRC_DEC_CODEC_MODE_UNDEFINED;

    if h_drc_dec.functional_range & DRC_DEC_SELECTION != 0 {
        let s_err = drc_dec_selection_process_create(&mut h_drc_dec.h_selection_proc);
        if s_err != 0 {
            return DRC_DEC_OUT_OF_MEMORY;
        }
        let s_err = drc_dec_selection_process_init(h_drc_dec.h_selection_proc.as_deref_mut());
        if s_err != 0 {
            return DRC_DEC_NOT_OK;
        }
        h_drc_dec.sel_proc_input_diff = 1;
    }

    if h_drc_dec.functional_range & DRC_DEC_GAIN != 0 {
        let d_err = drc_dec_gain_decoder_open(&mut h_drc_dec.h_gain_dec);
        if d_err != 0 {
            return DRC_DEC_OUT_OF_MEMORY;
        }
    }

    DRC_DEC_OK
}

/// `FDK_drcDec_SetCodecMode(hDrcDec, codecMode)`
pub fn fdk_drc_dec_set_codec_mode(
    h_drc_dec: Option<&mut DrcDecoder>,
    codec_mode: DrcDecCodecMode,
) -> DrcDecError {
    let Some(h_drc_dec) = h_drc_dec else {
        return DRC_DEC_NOT_OPENED;
    };

    if h_drc_dec.codec_mode == DRC_DEC_CODEC_MODE_UNDEFINED {
        /* Set codec mode, if it is set for the first time */
        h_drc_dec.codec_mode = codec_mode;

        if h_drc_dec.functional_range & DRC_DEC_SELECTION != 0 {
            let s_err = drc_dec_selection_process_set_codec_mode(
                h_drc_dec.h_selection_proc.as_deref_mut(),
                codec_mode as SelProcCodecMode,
            );
            if s_err != 0 {
                return DRC_DEC_NOT_OK;
            }
            h_drc_dec.sel_proc_input_diff = 1;
        }

        if h_drc_dec.functional_range & DRC_DEC_GAIN != 0 {
            let delay_mode = DM_REGULAR_DELAY;
            let time_domain_supported;
            let subband_domain_supported;

            match h_drc_dec.codec_mode {
                DRC_DEC_MPEG_4_AAC | DRC_DEC_MPEG_D_USAC => {
                    time_domain_supported = 1;
                    subband_domain_supported = SDM_OFF;
                }
                DRC_DEC_MPEG_H_3DA => {
                    time_domain_supported = 1;
                    subband_domain_supported = SDM_STFT256;
                }

                DRC_DEC_TEST_TIME_DOMAIN => {
                    time_domain_supported = 1;
                    subband_domain_supported = SDM_OFF;
                }
                DRC_DEC_TEST_QMF_DOMAIN => {
                    time_domain_supported = 0;
                    subband_domain_supported = SDM_QMF64;
                }
                DRC_DEC_TEST_STFT_DOMAIN => {
                    time_domain_supported = 0;
                    subband_domain_supported = SDM_STFT256;
                }

                _ => {
                    time_domain_supported = 0;
                    subband_domain_supported = SDM_OFF;
                }
            }

            if let Some(h_gain_dec) = h_drc_dec.h_gain_dec.as_deref_mut() {
                let d_err = drc_dec_gain_decoder_set_codec_dependent_parameters(
                    h_gain_dec,
                    delay_mode,
                    time_domain_supported,
                    subband_domain_supported,
                );
                if d_err != 0 {
                    return DRC_DEC_NOT_OK;
                }
            }
        }
    }

    /* Don't allow changing codecMode if it has already been set. */
    if h_drc_dec.codec_mode != codec_mode {
        return DRC_DEC_NOT_OK;
    }

    DRC_DEC_OK
}

/// `FDK_drcDec_Init(hDrcDec, frameSize, sampleRate, baseChannelCount)`
pub fn fdk_drc_dec_init(
    h_drc_dec: Option<&mut DrcDecoder>,
    frame_size: i32,
    sample_rate: i32,
    base_channel_count: i32,
) -> DrcDecError {
    let Some(h_drc_dec) = h_drc_dec else {
        return DRC_DEC_OK; /* return without doing anything */
    };
    if frame_size == 0 || sample_rate == 0 || base_channel_count == 0 {
        return DRC_DEC_OK; /* return without doing anything */
    }

    if h_drc_dec.functional_range & DRC_DEC_SELECTION != 0 {
        if let Some(sp) = h_drc_dec.h_selection_proc.as_deref_mut() {
            let s_err = drc_dec_selection_process_set_param(
                sp,
                SEL_PROC_BASE_CHANNEL_COUNT,
                base_channel_count,
                Some(&mut h_drc_dec.sel_proc_input_diff),
            );
            if s_err != 0 {
                return DRC_DEC_NOT_OK;
            }
            let s_err = drc_dec_selection_process_set_param(
                sp,
                SEL_PROC_SAMPLE_RATE,
                sample_rate,
                Some(&mut h_drc_dec.sel_proc_input_diff),
            );
            if s_err != 0 {
                return DRC_DEC_NOT_OK;
            }
        }
    }

    if h_drc_dec.functional_range & DRC_DEC_GAIN != 0 {
        if let Some(h_gain_dec) = h_drc_dec.h_gain_dec.as_deref_mut() {
            let mut d_err =
                drc_dec_gain_decoder_set_param(h_gain_dec, GAIN_DEC_FRAME_SIZE, frame_size);
            if d_err != 0 {
                return DRC_DEC_NOT_OK;
            }
            d_err = drc_dec_gain_decoder_set_param(h_gain_dec, GAIN_DEC_SAMPLE_RATE, sample_rate);
            if d_err != 0 {
                return DRC_DEC_NOT_OK;
            }
            d_err = drc_dec_gain_decoder_init(h_gain_dec);
            if d_err != 0 {
                return DRC_DEC_NOT_OK;
            }
        }
    }

    h_drc_dec.status = DRC_DEC_INITIALIZED;

    start_selection_process(h_drc_dec);

    DRC_DEC_OK
}

/// `FDK_drcDec_Close(phDrcDec)`
pub fn fdk_drc_dec_close(ph_drc_dec: Option<&mut HandleDrcDecoder>) -> DrcDecError {
    let Some(ph_drc_dec) = ph_drc_dec else {
        return DRC_DEC_OK;
    };

    let Some(h_drc_dec) = ph_drc_dec.as_deref_mut() else {
        return DRC_DEC_NOT_OPENED;
    };

    if h_drc_dec.functional_range & DRC_DEC_GAIN != 0 {
        drc_dec_gain_decoder_close(&mut h_drc_dec.h_gain_dec);
    }

    if h_drc_dec.functional_range & DRC_DEC_SELECTION != 0 {
        drc_dec_selection_process_delete(&mut h_drc_dec.h_selection_proc);
    }

    *ph_drc_dec = None;

    DRC_DEC_OK
}

/// `FDK_drcDec_SetParam(hDrcDec, requestType, requestValue)`: set single user request
pub fn fdk_drc_dec_set_param(
    h_drc_dec: Option<&mut DrcDecoder>,
    request_type: DrcDecUserParam,
    request_value: FixpDbl,
) -> DrcDecError {
    let mut invalid_parameter = 0;

    let Some(h_drc_dec) = h_drc_dec else {
        return DRC_DEC_NOT_OPENED;
    };

    if h_drc_dec.functional_range & DRC_DEC_GAIN != 0 {
        match request_type {
            DRC_DEC_SAMPLE_RATE => {
                if let Some(h_gain_dec) = h_drc_dec.h_gain_dec.as_deref_mut() {
                    let d_err = drc_dec_gain_decoder_set_param(
                        h_gain_dec,
                        GAIN_DEC_SAMPLE_RATE,
                        request_value,
                    );
                    if d_err != 0 {
                        return DRC_DEC_PARAM_OUT_OF_RANGE;
                    }
                }
            }
            DRC_DEC_FRAME_SIZE => {
                if let Some(h_gain_dec) = h_drc_dec.h_gain_dec.as_deref_mut() {
                    let d_err = drc_dec_gain_decoder_set_param(
                        h_gain_dec,
                        GAIN_DEC_FRAME_SIZE,
                        request_value,
                    );
                    if d_err != 0 {
                        return DRC_DEC_PARAM_OUT_OF_RANGE;
                    }
                }
            }
            _ => invalid_parameter |= DRC_DEC_GAIN,
        }
    }

    if h_drc_dec.functional_range & DRC_DEC_SELECTION != 0 {
        let sel_param = match request_type {
            DRC_DEC_BOOST => Some((SEL_PROC_BOOST, DRC_DEC_PARAM_OUT_OF_RANGE)),
            DRC_DEC_COMPRESS => Some((SEL_PROC_COMPRESS, DRC_DEC_PARAM_OUT_OF_RANGE)),
            DRC_DEC_LOUDNESS_NORMALIZATION_ON => {
                Some((SEL_PROC_LOUDNESS_NORMALIZATION_ON, DRC_DEC_PARAM_OUT_OF_RANGE))
            }
            DRC_DEC_TARGET_LOUDNESS => Some((SEL_PROC_TARGET_LOUDNESS, DRC_DEC_PARAM_OUT_OF_RANGE)),
            DRC_DEC_EFFECT_TYPE => Some((SEL_PROC_EFFECT_TYPE, DRC_DEC_PARAM_OUT_OF_RANGE)),
            DRC_DEC_DOWNMIX_ID => Some((SEL_PROC_DOWNMIX_ID, DRC_DEC_PARAM_OUT_OF_RANGE)),
            DRC_DEC_TARGET_CHANNEL_COUNT_REQUESTED => {
                Some((SEL_PROC_TARGET_CHANNEL_COUNT, DRC_DEC_PARAM_OUT_OF_RANGE))
            }
            DRC_DEC_BASE_CHANNEL_COUNT => Some((SEL_PROC_BASE_CHANNEL_COUNT, DRC_DEC_NOT_OK)),
            DRC_DEC_LOUDNESS_MEASUREMENT_METHOD => {
                Some((SEL_PROC_LOUDNESS_MEASUREMENT_METHOD, DRC_DEC_PARAM_OUT_OF_RANGE))
            }
            DRC_DEC_ALBUM_MODE => Some((SEL_PROC_ALBUM_MODE, DRC_DEC_PARAM_OUT_OF_RANGE)),
            _ => None,
        };
        match sel_param {
            Some((sel_type, err_ret)) => {
                if let Some(sp) = h_drc_dec.h_selection_proc.as_deref_mut() {
                    let s_err = drc_dec_selection_process_set_param(
                        sp,
                        sel_type,
                        request_value,
                        Some(&mut h_drc_dec.sel_proc_input_diff),
                    );
                    if s_err != 0 {
                        return err_ret;
                    }
                }
            }
            None => invalid_parameter |= DRC_DEC_SELECTION,
        }
    }

    if invalid_parameter == h_drc_dec.functional_range {
        return DRC_DEC_INVALID_PARAM;
    }

    /* All parameters need a new start of the selection process */
    start_selection_process(h_drc_dec);

    DRC_DEC_OK
}

/// `FDK_drcDec_GetParam(hDrcDec, requestType)`
///
/// For `DRC_DEC_IS_ACTIVE` on a decoder without selection process (`DRC_DEC_GAIN`) the port
/// takes the switches as off.
pub fn fdk_drc_dec_get_param(h_drc_dec: Option<&DrcDecoder>, request_type: DrcDecUserParam) -> i32 {
    let Some(h_drc_dec) = h_drc_dec else {
        return DRC_DEC_NOT_OPENED;
    };

    match request_type {
        DRC_DEC_BOOST => h_drc_dec.sel_proc_output.boost as i32,
        DRC_DEC_COMPRESS => h_drc_dec.sel_proc_output.compress as i32,
        DRC_DEC_IS_MULTIBAND_DRC_1 => {
            bitstream_contains_multiband_drc(&h_drc_dec.uni_drc_config, 0)
        }
        DRC_DEC_IS_MULTIBAND_DRC_2 => {
            bitstream_contains_multiband_drc(&h_drc_dec.uni_drc_config, 0x7F)
        }
        DRC_DEC_IS_ACTIVE => {
            /* MPEG-D DRC is considered active (and overrides MPEG-4 DRC), if uniDrc payload is
             * present (loudnessInfoSet and/or uniDrcConfig) at least one of DRC and Loudness
             * Control is switched on */
            let (drc_on, ln_on) = match h_drc_dec.h_selection_proc.as_deref() {
                Some(sp) => (
                    drc_dec_selection_process_get_param(sp, SEL_PROC_DYNAMIC_RANGE_CONTROL_ON),
                    drc_dec_selection_process_get_param(sp, SEL_PROC_LOUDNESS_NORMALIZATION_ON),
                ),
                None => (0, 0),
            };
            let mut uni_drc_payload_present =
                (h_drc_dec.loudness_info_set.loudness_info_count > 0) as i32;
            uni_drc_payload_present |=
                (h_drc_dec.loudness_info_set.loudness_info_album_count > 0) as i32;
            uni_drc_payload_present |=
                (h_drc_dec.uni_drc_config.drc_instructions_uni_drc_count > 0) as i32;
            uni_drc_payload_present |=
                (h_drc_dec.uni_drc_config.downmix_instructions_count > 0) as i32;
            (uni_drc_payload_present != 0 && (drc_on != 0 || ln_on != 0)) as i32
        }
        DRC_DEC_TARGET_CHANNEL_COUNT_SELECTED => {
            h_drc_dec.sel_proc_output.target_channel_count as i32
        }
        DRC_DEC_OUTPUT_LOUDNESS => h_drc_dec.sel_proc_output.output_loudness,
        _ => 0,
    }
}

/// `FDK_drcDec_SetInterfaceParameters(hDrcDec, uniDrcInterface)`: not supported.
pub fn fdk_drc_dec_set_interface_parameters(_h_drc_dec: Option<&mut DrcDecoder>) -> DrcDecError {
    DRC_DEC_UNSUPPORTED_FUNCTION
}

/// `FDK_drcDec_SetSelectionProcessMpeghParameters_simple(hDrcDec, groupPresetIdRequested,
/// numGroupIdsRequested, groupIdsRequested)`: not supported.
pub fn fdk_drc_dec_set_selection_process_mpegh_parameters_simple(
    _h_drc_dec: Option<&mut DrcDecoder>,
    _group_preset_id_requested: i32,
    _num_group_ids_requested: i32,
    _group_ids_requested: &[i32],
) -> DrcDecError {
    DRC_DEC_UNSUPPORTED_FUNCTION
}

/// `FDK_drcDec_SetDownmixInstructions(hDrcDec, numDownmixId, downmixId, targetLayout,
/// targetChannelCount)`: not supported.
pub fn fdk_drc_dec_set_downmix_instructions(
    _h_drc_dec: Option<&mut DrcDecoder>,
    _num_downmix_id: i32,
    _downmix_id: &[i32],
    _target_layout: &[i32],
    _target_channel_count: &[i32],
) -> DrcDecError {
    DRC_DEC_UNSUPPORTED_FUNCTION
}

/// `FDK_drcDec_SetSelectionProcessOutput(hDrcDec, hSelProcOutput)`: does nothing.
pub fn fdk_drc_dec_set_selection_process_output(
    _h_drc_dec: Option<&mut DrcDecoder>,
    _h_sel_proc_output: Option<&SelProcOutput>,
) {
}

/// `FDK_drcDec_GetSelectionProcessOutput(hDrcDec)`
pub fn fdk_drc_dec_get_selection_process_output(
    h_drc_dec: Option<&DrcDecoder>,
) -> HandleSelProcOutput<'_> {
    let h_drc_dec = h_drc_dec?;

    Some(&h_drc_dec.sel_proc_output)
}

/// `FDK_drcDec_GetGroupLoudness(hSelProcOutput, groupID, groupLoudnessAvailable)`: always 0
/// (`groupLoudnessAvailable` is not written).
pub fn fdk_drc_dec_get_group_loudness(
    _h_sel_proc_output: Option<&SelProcOutput>,
    _group_id: i32,
    _group_loudness_available: &mut i32,
) -> i32 {
    0
}

/// `FDK_drcDec_SetChannelGains(hDrcDec, numChannels, frameSize, channelGainDb, audioBuffer,
/// audioBufferChannelOffset)`
///
/// For a decoder without gain decoder (`DRC_DEC_SELECTION`) the port returns.
pub fn fdk_drc_dec_set_channel_gains(
    h_drc_dec: Option<&mut DrcDecoder>,
    num_channels: i32,
    frame_size: i32,
    channel_gain_db: &[FixpDbl],
    audio_buffer: &mut [FixpDbl],
    audio_buffer_channel_offset: i32,
) {
    let Some(h_drc_dec) = h_drc_dec else {
        return;
    };
    let Some(h_gain_dec) = h_drc_dec.h_gain_dec.as_deref_mut() else {
        return;
    };

    let err = drc_dec_gain_decoder_set_loudness_normalization_gain_db(
        h_gain_dec,
        h_drc_dec.sel_proc_output.loudness_normalization_gain_db,
    );
    if err != 0 {
        return;
    }

    drc_dec_gain_decoder_set_channel_gains(
        h_gain_dec,
        num_channels,
        frame_size,
        channel_gain_db,
        audio_buffer_channel_offset,
        audio_buffer,
    );
}

/// `FDK_drcDec_ReadUniDrcConfig(hDrcDec, hBitstream)`
pub fn fdk_drc_dec_read_uni_drc_config(
    h_drc_dec: Option<&mut DrcDecoder>,
    h_bitstream: &mut FdkBitStream,
) -> DrcDecError {
    let Some(h_drc_dec) = h_drc_dec else {
        return DRC_DEC_NOT_OPENED;
    };

    let d_err = if h_drc_dec.codec_mode == DRC_DEC_MPEG_D_USAC {
        drc_dec_read_uni_drc_config(h_bitstream, &mut h_drc_dec.uni_drc_config)
    } else {
        return DRC_DEC_NOT_OK;
    };

    if d_err != 0 {
        /* clear config, if parsing error occured */
        *h_drc_dec.uni_drc_config = UniDrcConfig::ZERO;
        h_drc_dec.uni_drc_config.diff = 1;
    }

    start_selection_process(h_drc_dec);

    DRC_DEC_OK
}

/// `FDK_drcDec_ReadDownmixInstructions_Box(hDrcDec, hBitstream)`: not supported (returns
/// `DRC_DEC_NOT_OK`).
pub fn fdk_drc_dec_read_downmix_instructions_box(
    h_drc_dec: Option<&mut DrcDecoder>,
    _h_bitstream: &mut FdkBitStream,
) -> DrcDecError {
    if h_drc_dec.is_none() {
        return DRC_DEC_NOT_OPENED;
    }

    DRC_DEC_NOT_OK
}

/// `FDK_drcDec_ReadUniDrcInstructions_Box(hDrcDec, hBitstream)`: not supported (returns
/// `DRC_DEC_NOT_OK`).
pub fn fdk_drc_dec_read_uni_drc_instructions_box(
    h_drc_dec: Option<&mut DrcDecoder>,
    _h_bitstream: &mut FdkBitStream,
) -> DrcDecError {
    if h_drc_dec.is_none() {
        return DRC_DEC_NOT_OPENED;
    }

    DRC_DEC_NOT_OK
}

/// `FDK_drcDec_ReadUniDrcCoefficients_Box(hDrcDec, hBitstream)`: not supported (returns
/// `DRC_DEC_NOT_OK`).
pub fn fdk_drc_dec_read_uni_drc_coefficients_box(
    h_drc_dec: Option<&mut DrcDecoder>,
    _h_bitstream: &mut FdkBitStream,
) -> DrcDecError {
    if h_drc_dec.is_none() {
        return DRC_DEC_NOT_OPENED;
    }

    DRC_DEC_NOT_OK
}

/// `FDK_drcDec_ReadLoudnessInfoSet(hDrcDec, hBitstream)`
pub fn fdk_drc_dec_read_loudness_info_set(
    h_drc_dec: Option<&mut DrcDecoder>,
    h_bitstream: &mut FdkBitStream,
) -> DrcDecError {
    let Some(h_drc_dec) = h_drc_dec else {
        return DRC_DEC_NOT_OPENED;
    };

    let d_err = if h_drc_dec.codec_mode == DRC_DEC_MPEG_D_USAC {
        drc_dec_read_loudness_info_set(h_bitstream, &mut h_drc_dec.loudness_info_set)
    } else {
        return DRC_DEC_NOT_OK;
    };

    if d_err != 0 {
        /* clear config, if parsing error occurred */
        *h_drc_dec.loudness_info_set = LoudnessInfoSet::ZERO;
        h_drc_dec.loudness_info_set.diff = 1;
    }

    start_selection_process(h_drc_dec);

    DRC_DEC_OK
}

/// `FDK_drcDec_ReadLoudnessBox(hDrcDec, hBitstream)`: not supported (returns
/// `DRC_DEC_NOT_OK`).
pub fn fdk_drc_dec_read_loudness_box(
    h_drc_dec: Option<&mut DrcDecoder>,
    _h_bitstream: &mut FdkBitStream,
) -> DrcDecError {
    if h_drc_dec.is_none() {
        return DRC_DEC_NOT_OPENED;
    }

    DRC_DEC_NOT_OK
}

/// `FDK_drcDec_ReadUniDrcGain(hDrcDec, hBitstream)`
pub fn fdk_drc_dec_read_uni_drc_gain(
    h_drc_dec: Option<&mut DrcDecoder>,
    h_bitstream: &mut FdkBitStream,
) -> DrcDecError {
    let Some(h_drc_dec) = h_drc_dec else {
        return DRC_DEC_NOT_OPENED;
    };
    if h_drc_dec.status == 0 {
        return DRC_DEC_OK;
    }

    let d_err = drc_dec_read_uni_drc_gain(
        h_bitstream,
        &h_drc_dec.uni_drc_config,
        drc_dec_gain_decoder_get_frame_size(h_drc_dec.h_gain_dec.as_deref()),
        drc_dec_gain_decoder_get_delta_tmin_default(h_drc_dec.h_gain_dec.as_deref()),
        &mut h_drc_dec.uni_drc_gain,
    );
    if d_err != 0 {
        return DRC_DEC_NOT_OK;
    }

    if get_gain_status(&h_drc_dec.uni_drc_gain) != 0 {
        h_drc_dec.status = DRC_DEC_NEW_GAIN_PAYLOAD;
    }

    DRC_DEC_OK
}

/// `FDK_drcDec_ReadUniDrc(hDrcDec, hBitstream)`: reads uniDrc() (loudnessInfoSet,
/// uniDrcConfig and uniDrcGain)
pub fn fdk_drc_dec_read_uni_drc(
    h_drc_dec: Option<&mut DrcDecoder>,
    h_bitstream: &mut FdkBitStream,
) -> DrcDecError {
    let Some(h_drc_dec) = h_drc_dec else {
        return DRC_DEC_NOT_OPENED;
    };
    if h_drc_dec.status == 0 {
        return DRC_DEC_NOT_READY;
    }

    let frame_size = drc_dec_gain_decoder_get_frame_size(h_drc_dec.h_gain_dec.as_deref());
    let delta_tmin = drc_dec_gain_decoder_get_delta_tmin_default(h_drc_dec.h_gain_dec.as_deref());
    let d_err = drc_dec_read_uni_drc(
        h_bitstream,
        &mut h_drc_dec.uni_drc_config,
        &mut h_drc_dec.loudness_info_set,
        frame_size,
        delta_tmin,
        &mut h_drc_dec.uni_drc_gain,
    );

    start_selection_process(h_drc_dec);
    if d_err != 0 {
        return DRC_DEC_NOT_OK;
    }

    if get_gain_status(&h_drc_dec.uni_drc_gain) != 0 {
        h_drc_dec.status = DRC_DEC_NEW_GAIN_PAYLOAD;
    }

    DRC_DEC_OK
}

/// `FDK_drcDec_Preprocess(hDrcDec)`
pub fn fdk_drc_dec_preprocess(h_drc_dec: Option<&mut DrcDecoder>) -> DrcDecError {
    let Some(h_drc_dec) = h_drc_dec else {
        return DRC_DEC_NOT_OPENED;
    };
    if h_drc_dec.status == 0 {
        return DRC_DEC_NOT_READY;
    }
    if h_drc_dec.functional_range & DRC_DEC_GAIN == 0 {
        return DRC_DEC_NOT_OK;
    }
    let Some(h_gain_dec) = h_drc_dec.h_gain_dec.as_deref_mut() else {
        return DRC_DEC_NOT_OK;
    };

    if h_drc_dec.status != DRC_DEC_NEW_GAIN_PAYLOAD {
        /* no new gain payload was read, e.g. during concalment or flushing. Generate DRC gains
        based on the stored DRC gains of last frames */
        drc_dec_gain_decoder_conceal(
            h_gain_dec,
            &h_drc_dec.uni_drc_config,
            &mut h_drc_dec.uni_drc_gain,
        );
    }

    let d_err = drc_dec_gain_decoder_preprocess(
        h_gain_dec,
        &h_drc_dec.uni_drc_config,
        &h_drc_dec.uni_drc_gain,
        h_drc_dec.sel_proc_output.loudness_normalization_gain_db,
        h_drc_dec.sel_proc_output.boost,
        h_drc_dec.sel_proc_output.compress,
    );
    if d_err != 0 {
        return DRC_DEC_NOT_OK;
    }
    h_drc_dec.status = DRC_DEC_INTERPOLATION_PREPARED;

    DRC_DEC_OK
}

/// `FDK_drcDec_ProcessTime(hDrcDec, delaySamples, drcLocation, channelOffset,
/// drcChannelOffset, numChannelsProcessed, realBuffer, timeDataChannelOffset)`
pub fn fdk_drc_dec_process_time(
    h_drc_dec: Option<&mut DrcDecoder>,
    delay_samples: i32,
    drc_location: DrcDecLocation,
    channel_offset: i32,
    drc_channel_offset: i32,
    num_channels_processed: i32,
    real_buffer: &mut [FixpDbl],
    time_data_channel_offset: i32,
) -> DrcDecError {
    let Some(h_drc_dec) = h_drc_dec else {
        return DRC_DEC_NOT_OPENED;
    };
    if h_drc_dec.functional_range & DRC_DEC_GAIN == 0 {
        return DRC_DEC_NOT_OK;
    }
    if h_drc_dec.status != DRC_DEC_INTERPOLATION_PREPARED {
        return DRC_DEC_NOT_READY;
    }
    let Some(h_gain_dec) = h_drc_dec.h_gain_dec.as_deref_mut() else {
        return DRC_DEC_NOT_OK;
    };

    let d_err = drc_dec_gain_decoder_process_time_domain(
        h_gain_dec,
        &h_drc_dec.uni_drc_config,
        delay_samples,
        drc_location as GainDecLocation,
        channel_offset,
        drc_channel_offset,
        num_channels_processed,
        time_data_channel_offset,
        real_buffer,
    );
    if d_err != 0 {
        return DRC_DEC_NOT_OK;
    }

    DRC_DEC_OK
}

/// `FDK_drcDec_ProcessFreq(hDrcDec, delaySamples, drcLocation, channelOffset,
/// drcChannelOffset, numChannelsProcessed, processSingleTimeslot, realBuffer, imagBuffer)`
pub fn fdk_drc_dec_process_freq(
    h_drc_dec: Option<&mut DrcDecoder>,
    delay_samples: i32,
    drc_location: DrcDecLocation,
    channel_offset: i32,
    drc_channel_offset: i32,
    num_channels_processed: i32,
    process_single_timeslot: i32,
    real_buffer: &mut [&mut [FixpDbl]],
    imag_buffer: &mut [&mut [FixpDbl]],
) -> DrcDecError {
    let Some(h_drc_dec) = h_drc_dec else {
        return DRC_DEC_NOT_OPENED;
    };
    if h_drc_dec.functional_range & DRC_DEC_GAIN == 0 {
        return DRC_DEC_NOT_OK;
    }
    if h_drc_dec.status != DRC_DEC_INTERPOLATION_PREPARED {
        return DRC_DEC_NOT_READY;
    }
    let Some(h_gain_dec) = h_drc_dec.h_gain_dec.as_deref_mut() else {
        return DRC_DEC_NOT_OK;
    };

    let d_err = drc_dec_gain_decoder_process_subband_domain(
        h_gain_dec,
        &h_drc_dec.uni_drc_config,
        delay_samples,
        drc_location as GainDecLocation,
        channel_offset,
        drc_channel_offset,
        num_channels_processed,
        process_single_timeslot,
        real_buffer,
        imag_buffer,
    );
    if d_err != 0 {
        return DRC_DEC_NOT_OK;
    }

    DRC_DEC_OK
}

/// `FDK_drcDec_ApplyDownmix(hDrcDec, reverseInChannelMap, reverseOutChannelMap, realBuffer,
/// pNChannels)`
///
/// Without `hDrcDec` the port returns `DRC_DEC_NOT_OPENED`. Channel map entries outside of 0..8
/// get a downmix coefficient of 0. Samples outside of `realBuffer` are skipped.
pub fn fdk_drc_dec_apply_downmix(
    h_drc_dec: Option<&mut DrcDecoder>,
    reverse_in_channel_map: Option<&[i32]>,
    reverse_out_channel_map: Option<&[i32]>,
    real_buffer: Option<&mut [FixpDbl]>,
    p_n_channels: &mut i32,
) -> DrcDecError {
    let Some(h_drc_dec) = h_drc_dec else {
        return DRC_DEC_NOT_OPENED;
    };
    let p_sel_proc_output = &h_drc_dec.sel_proc_output;
    let base_ch_cnt = p_sel_proc_output.base_channel_count as i32;
    let target_ch_cnt = p_sel_proc_output.target_channel_count as i32;
    let mut tmp_out = [0 as FixpDbl; 8];

    if h_drc_dec.functional_range & DRC_DEC_GAIN == 0 {
        return DRC_DEC_NOT_OK;
    }

    /* only downmix is performed here, no upmix. Downmix is only performed if downmix
    coefficients are provided. All other cases of downmix and upmix are treated by pcmDmx
    library. */
    if p_sel_proc_output.downmix_matrix_present == 0 {
        return DRC_DEC_OK; /* no downmix */
    }
    if target_ch_cnt >= base_ch_cnt {
        return DRC_DEC_OK; /* downmix only */
    }

    /* sanity checks */
    let Some(real_buffer) = real_buffer else {
        return DRC_DEC_NOT_OK;
    };
    let Some(reverse_in_channel_map) = reverse_in_channel_map else {
        return DRC_DEC_NOT_OK;
    };
    let Some(reverse_out_channel_map) = reverse_out_channel_map else {
        return DRC_DEC_NOT_OK;
    };
    if base_ch_cnt > 8 {
        return DRC_DEC_NOT_OK;
    }
    if base_ch_cnt != *p_n_channels {
        return DRC_DEC_NOT_OK;
    }
    if target_ch_cnt > 8 {
        return DRC_DEC_NOT_OK;
    }

    let frame_size = drc_dec_gain_decoder_get_frame_size(h_drc_dec.h_gain_dec.as_deref());

    let base = base_ch_cnt as usize;
    let target = target_ch_cnt as usize;
    let coef = |ic: usize, oc: usize| -> FixpDbl {
        let i = reverse_in_channel_map.get(ic).copied().unwrap_or(-1);
        let o = reverse_out_channel_map.get(oc).copied().unwrap_or(-1);
        if (0..8).contains(&i) && (0..8).contains(&o) {
            p_sel_proc_output.downmix_matrix[i as usize][o as usize]
        } else {
            0
        }
    };
    let pos = |ch: usize, n: i32| -> Option<usize> {
        usize::try_from(ch as i64 * frame_size as i64 + n as i64).ok()
    };

    /* in-place downmix */
    for n in 0..frame_size.max(0) {
        for oc in 0..target {
            tmp_out[oc] = 0;
            for ic in 0..base {
                let x = pos(ic, n).and_then(|k| real_buffer.get(k)).copied().unwrap_or(0);
                tmp_out[oc] =
                    tmp_out[oc].wrapping_add(f_mult_div2(x, coef(ic, oc)).wrapping_shl(3));
            }
        }
        for oc in 0..target {
            if oc >= base {
                break;
            }
            if let Some(s) = pos(oc, n).and_then(|k| real_buffer.get_mut(k)) {
                *s = tmp_out[oc];
            }
        }
    }

    for oc in target..base {
        for n in 0..frame_size.max(0) {
            if let Some(s) = pos(oc, n).and_then(|k| real_buffer.get_mut(k)) {
                *s = 0;
            }
        }
    }

    *p_n_channels = target_ch_cnt;

    DRC_DEC_OK
}

/// `FDK_drcDec_GetLibInfo(info)`: get library info for this module.
///
/// `build_date` and `build_time` are empty; `flags` is not written, as in C.
pub fn fdk_drc_dec_get_lib_info(info: Option<&mut [LibInfo]>) -> DrcDecError {
    let Some(info) = info else {
        return DRC_DEC_INVALID_PARAM;
    };

    /* Search for next free tab */
    let mut i = 0;
    while i < FDK_MODULE_LAST as usize {
        // The C++ code reads FDK_MODULE_LAST entries; a shorter slice ends the search.
        let Some(e) = info.get(i) else {
            return DRC_DEC_NOT_OK;
        };
        if e.module_id == FDK_NONE {
            break;
        }
        i += 1;
    }
    if i == FDK_MODULE_LAST as usize {
        return DRC_DEC_NOT_OK;
    }

    /* Add the library info */
    let e = &mut info[i];
    e.module_id = FDK_UNIDRCDEC;
    e.version = lib_version(DRCDEC_LIB_VL0, DRCDEC_LIB_VL1, DRCDEC_LIB_VL2);
    lib_version_string(e);
    e.build_date = DRCDEC_LIB_BUILD_DATE;
    e.build_time = DRCDEC_LIB_BUILD_TIME;
    e.title = DRCDEC_LIB_TITLE;

    DRC_DEC_OK
}
