// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libDRCdec/src/drcDec_selectionProcess.h,
// libDRCdec/src/drcDec_selectionProcess.cpp.

//! Port of `drcDec_selectionProcess.h`: DRC set selection according to section 6.2 of
//! ISO/IEC 23003-4 (MPEG-D DRC), including ISO/IEC 23003-4/AMD1 (Amendment 1).
//!
//! The structures have the memory layout of the C structures (see [`crate::drc_dec_types`]).
//! The pointer `pInst` of `DRCDEC_SELECTION_DATA` points into the `UNI_DRC_CONFIG`; it is stored
//! as the index of the element plus one (0 for `NULL`) in a 64 bit field at the place of the
//! pointer. The two candidate lists, which C addresses through two swapped pointers, are the
//! indices of the two `selectionData` entries.
//!
//! `drcDec_SelectionProcess_SetMpeghParams` is declared in the header but not defined in the
//! reference; it is not ported.

use fdk_aac_rs_fdk::common_fix::{
    f_abs, f_max, f_min, f_mult_div2, fl2fxconst_dbl, fl2fxconst_sgl,
};
use fdk_aac_rs_fdk::common_fix::{fx_dbl2fx_sgl, FixpDbl, FixpSgl};
use fdk_aac_rs_fdk::common_fix::{DFRACT_BITS, MAXVAL_DBL, MINVAL_DBL};
use fdk_aac_rs_fdk::scale::{saturate_left_shift, scale_value};

use crate::drc_dec_reader::comp_assign;
use crate::drc_dec_tools::*;
use crate::drc_dec_types::*;
use crate::drc_decoder::*;

/// `HANDLE_DRC_SELECTION_PROCESS`
pub type HandleDrcSelectionProcess = Option<Box<DrcSelectionProcess>>;

/// `UNDEFINED_LOUDNESS_VALUE`
pub const UNDEFINED_LOUDNESS_VALUE: FixpDbl = MAXVAL_DBL - 1;

/// `DRCDEC_SELECTION_PROCESS_RETURN`
pub type DrcdecSelectionProcessReturn = i32;
/// `DRCDEC_SELECTION_PROCESS_NO_ERROR`
pub const DRCDEC_SELECTION_PROCESS_NO_ERROR: DrcdecSelectionProcessReturn = 0;
/// `DRCDEC_SELECTION_PROCESS_WARNING`
pub const DRCDEC_SELECTION_PROCESS_WARNING: DrcdecSelectionProcessReturn = -1000;
/// `DRCDEC_SELECTION_PROCESS_NOT_OK`
pub const DRCDEC_SELECTION_PROCESS_NOT_OK: DrcdecSelectionProcessReturn = -2000;
/// `DRCDEC_SELECTION_PROCESS_OUTOFMEMORY`
pub const DRCDEC_SELECTION_PROCESS_OUTOFMEMORY: DrcdecSelectionProcessReturn = -1999;
/// `DRCDEC_SELECTION_PROCESS_INVALID_HANDLE`
pub const DRCDEC_SELECTION_PROCESS_INVALID_HANDLE: DrcdecSelectionProcessReturn = -1998;
/// `DRCDEC_SELECTION_PROCESS_NOT_SUPPORTED`
pub const DRCDEC_SELECTION_PROCESS_NOT_SUPPORTED: DrcdecSelectionProcessReturn = -1997;
/// `DRCDEC_SELECTION_PROCESS_INVALID_PARAM`
pub const DRCDEC_SELECTION_PROCESS_INVALID_PARAM: DrcdecSelectionProcessReturn = -1996;
/// `DRCDEC_SELECTION_PROCESS_PARAM_OUT_OF_RANGE`
pub const DRCDEC_SELECTION_PROCESS_PARAM_OUT_OF_RANGE: DrcdecSelectionProcessReturn = -1995;

/// `SEL_PROC_CODEC_MODE`
pub type SelProcCodecMode = i32;
/// `SEL_PROC_TEST_TIME_DOMAIN`
pub const SEL_PROC_TEST_TIME_DOMAIN: SelProcCodecMode = -100;
/// `SEL_PROC_TEST_QMF_DOMAIN`
pub const SEL_PROC_TEST_QMF_DOMAIN: SelProcCodecMode = -99;
/// `SEL_PROC_TEST_STFT_DOMAIN`
pub const SEL_PROC_TEST_STFT_DOMAIN: SelProcCodecMode = -98;
/// `SEL_PROC_CODEC_MODE_UNDEFINED`
pub const SEL_PROC_CODEC_MODE_UNDEFINED: SelProcCodecMode = -1;
/// `SEL_PROC_MPEG_4_AAC`
pub const SEL_PROC_MPEG_4_AAC: SelProcCodecMode = 0;
/// `SEL_PROC_MPEG_D_USAC`
pub const SEL_PROC_MPEG_D_USAC: SelProcCodecMode = 1;
/// `SEL_PROC_MPEG_H_3DA`
pub const SEL_PROC_MPEG_H_3DA: SelProcCodecMode = 2;

/// `SEL_PROC_USER_PARAM`
pub type SelProcUserParam = i32;
/* set and get user param */
/// `SEL_PROC_LOUDNESS_NORMALIZATION_ON`
pub const SEL_PROC_LOUDNESS_NORMALIZATION_ON: SelProcUserParam = 0;
/* get only user param */
/// `SEL_PROC_DYNAMIC_RANGE_CONTROL_ON`
pub const SEL_PROC_DYNAMIC_RANGE_CONTROL_ON: SelProcUserParam = 1;
/* set only user params */
/// `SEL_PROC_TARGET_LOUDNESS`
pub const SEL_PROC_TARGET_LOUDNESS: SelProcUserParam = 2;
/// `SEL_PROC_EFFECT_TYPE`
pub const SEL_PROC_EFFECT_TYPE: SelProcUserParam = 3;
/// `SEL_PROC_EFFECT_TYPE_FALLBACK_CODE`
pub const SEL_PROC_EFFECT_TYPE_FALLBACK_CODE: SelProcUserParam = 4;
/// `SEL_PROC_LOUDNESS_MEASUREMENT_METHOD`
pub const SEL_PROC_LOUDNESS_MEASUREMENT_METHOD: SelProcUserParam = 5;
/// `SEL_PROC_ALBUM_MODE`
pub const SEL_PROC_ALBUM_MODE: SelProcUserParam = 6;
/// `SEL_PROC_DOWNMIX_ID`
pub const SEL_PROC_DOWNMIX_ID: SelProcUserParam = 7;
/// `SEL_PROC_TARGET_LAYOUT`
pub const SEL_PROC_TARGET_LAYOUT: SelProcUserParam = 8;
/// `SEL_PROC_TARGET_CHANNEL_COUNT`
pub const SEL_PROC_TARGET_CHANNEL_COUNT: SelProcUserParam = 9;
/// `SEL_PROC_BASE_CHANNEL_COUNT`
pub const SEL_PROC_BASE_CHANNEL_COUNT: SelProcUserParam = 10;
/// `SEL_PROC_SAMPLE_RATE`
pub const SEL_PROC_SAMPLE_RATE: SelProcUserParam = 11;
/// `SEL_PROC_BOOST`
pub const SEL_PROC_BOOST: SelProcUserParam = 12;
/// `SEL_PROC_COMPRESS`
pub const SEL_PROC_COMPRESS: SelProcUserParam = 13;

pod_struct! {
    /// `SEL_PROC_OUTPUT` (`struct s_selection_process_output`)
    pub struct SelProcOutput {
        /// e = 7
        pub output_peak_level_db: FixpDbl,
        /// e = 7
        pub loudness_normalization_gain_db: FixpDbl,
        /// e = 7
        pub output_loudness: FixpDbl,
        pub num_selected_drc_sets: u8,
        pub selected_drc_set_ids: [i8; MAX_ACTIVE_DRCS],
        pub selected_downmix_ids: [u8; MAX_ACTIVE_DRCS],
        pub active_downmix_id: u8,
        pub base_channel_count: u8,
        pub target_channel_count: u8,
        pub target_layout: i8,
        pub downmix_matrix_present: u8,
        /// e = 2
        pub downmix_matrix: [[FixpDbl; 8]; 8],
        /// e = 1
        pub boost: FixpSgl,
        /// e = 1
        pub compress: FixpSgl,
        /// e = 7
        pub mixing_level: FixpDbl,
    }
}

impl SelProcOutput {
    /// Assignment to `selectedDrcSetIds[i]` with the C addressing: index 3 reaches the next field
    /// of the C structure (`selectedDownmixIds[0]`); reproduced from the C layout.
    fn set_selected_drc_set_id(&mut self, i: usize, v: i8) {
        let off = core::mem::offset_of!(SelProcOutput, selected_drc_set_ids) + i;
        if let Some(b) = pod_bytes_mut(self).get_mut(off) {
            *b = v as u8;
        }
    }

    /// Assignment to `selectedDownmixIds[i]` with the C addressing: index 3 is
    /// `activeDownmixId`.
    fn set_selected_downmix_id(&mut self, i: usize, v: u8) {
        let off = core::mem::offset_of!(SelProcOutput, selected_downmix_ids) + i;
        if let Some(b) = pod_bytes_mut(self).get_mut(off) {
            *b = v;
        }
    }
}

/// `DRC_EFFECT_TYPE_REQUEST`
pub type DrcEffectTypeRequest = i32;
/// `DETR_NONE`
pub const DETR_NONE: DrcEffectTypeRequest = 0;
/// `DETR_NIGHT`
pub const DETR_NIGHT: DrcEffectTypeRequest = 1;
/// `DETR_NOISY`
pub const DETR_NOISY: DrcEffectTypeRequest = 2;
/// `DETR_LIMITED`
pub const DETR_LIMITED: DrcEffectTypeRequest = 3;
/// `DETR_LOWLEVEL`
pub const DETR_LOWLEVEL: DrcEffectTypeRequest = 4;
/// `DETR_DIALOG`
pub const DETR_DIALOG: DrcEffectTypeRequest = 5;
/// `DETR_GENERAL_COMPR`
pub const DETR_GENERAL_COMPR: DrcEffectTypeRequest = 6;
/// `DETR_EXPAND`
pub const DETR_EXPAND: DrcEffectTypeRequest = 7;
/// `DETR_ARTISTIC`
pub const DETR_ARTISTIC: DrcEffectTypeRequest = 8;
/// `DETR_COUNT`
pub const DETR_COUNT: DrcEffectTypeRequest = 9;

/// `DRC_FEATURE_REQUEST_TYPE`
pub type DrcFeatureRequestType = i32;
/// `DFRT_EFFECT_TYPE`
pub const DFRT_EFFECT_TYPE: DrcFeatureRequestType = 0;
/// `DFRT_DYNAMIC_RANGE`
pub const DFRT_DYNAMIC_RANGE: DrcFeatureRequestType = 1;
/// `DFRT_DRC_CHARACTERISTIC`
pub const DFRT_DRC_CHARACTERISTIC: DrcFeatureRequestType = 2;

/// `METHOD_DEFINITION_REQUEST`
pub type MethodDefinitionRequest = i32;
/// `MDR_DEFAULT`
pub const MDR_DEFAULT: MethodDefinitionRequest = 0;
/// `MDR_PROGRAM_LOUDNESS`
pub const MDR_PROGRAM_LOUDNESS: MethodDefinitionRequest = 1;
/// `MDR_ANCHOR_LOUDNESS`
pub const MDR_ANCHOR_LOUDNESS: MethodDefinitionRequest = 2;

/// `MEASUREMENT_SYSTEM_REQUEST`
pub type MeasurementSystemRequest = i32;
/// `MSR_DEFAULT`
pub const MSR_DEFAULT: MeasurementSystemRequest = 0;
/// `MSR_BS_1770_4`
pub const MSR_BS_1770_4: MeasurementSystemRequest = 1;
/// `MSR_USER`
pub const MSR_USER: MeasurementSystemRequest = 2;
/// `MSR_EXPERT_PANEL`
pub const MSR_EXPERT_PANEL: MeasurementSystemRequest = 3;
/// `MSR_RESERVED_A`
pub const MSR_RESERVED_A: MeasurementSystemRequest = 4;
/// `MSR_RESERVED_B`
pub const MSR_RESERVED_B: MeasurementSystemRequest = 5;
/// `MSR_RESERVED_C`
pub const MSR_RESERVED_C: MeasurementSystemRequest = 6;
/// `MSR_RESERVED_D`
pub const MSR_RESERVED_D: MeasurementSystemRequest = 7;
/// `MSR_RESERVED_E`
pub const MSR_RESERVED_E: MeasurementSystemRequest = 8;

/// `LOUDNESS_PREPROCESSING_REQUEST`
pub type LoudnessPreprocessingRequest = i32;
/// `LPR_DEFAULT`
pub const LPR_DEFAULT: LoudnessPreprocessingRequest = 0;
/// `LPR_OFF`
pub const LPR_OFF: LoudnessPreprocessingRequest = 1;
/// `LPR_HIGHPASS`
pub const LPR_HIGHPASS: LoudnessPreprocessingRequest = 2;

/// `DYN_RANGE_MEASUREMENT_REQUEST_TYPE`
pub type DynRangeMeasurementRequestType = i32;
/// `DRMRT_SHORT_TERM_LOUDNESS_TO_AVG`
pub const DRMRT_SHORT_TERM_LOUDNESS_TO_AVG: DynRangeMeasurementRequestType = 0;
/// `DRMRT_MOMENTARY_LOUDNESS_TO_AVG`
pub const DRMRT_MOMENTARY_LOUDNESS_TO_AVG: DynRangeMeasurementRequestType = 1;
/// `DRMRT_TOP_OF_LOUDNESS_RANGE_TO_AVG`
pub const DRMRT_TOP_OF_LOUDNESS_RANGE_TO_AVG: DynRangeMeasurementRequestType = 2;

/// `TARGET_CONFIG_REQUEST_TYPE`
pub type TargetConfigRequestType = i32;
/// `TCRT_DOWNMIX_ID`
pub const TCRT_DOWNMIX_ID: TargetConfigRequestType = 0;
/// `TCRT_TARGET_LAYOUT`
pub const TCRT_TARGET_LAYOUT: TargetConfigRequestType = 1;
/// `TCRT_TARGET_CHANNEL_COUNT`
pub const TCRT_TARGET_CHANNEL_COUNT: TargetConfigRequestType = 2;

pod_struct! {
    /// `drcEffectType` member of `DRC_FEATURE_REQUEST`
    pub struct DrcEffectTypeFeature {
        pub num_requests: u8,
        pub num_requests_desired: u8,
        pub _pad0: [u8; 2],
        pub request: [DrcEffectTypeRequest; MAX_REQUESTS_DRC_EFFECT_TYPE],
    }
}

pod_struct! {
    /// `dynamicRange` member of `DRC_FEATURE_REQUEST`
    pub struct DynamicRangeFeature {
        pub measurement_request_type: DynRangeMeasurementRequestType,
        pub requested_is_range: u8,
        pub _pad0: [u8; 3],
        /// e = 7
        pub request_value: FixpDbl,
        /// e = 7
        pub request_value_min: FixpDbl,
        /// e = 7
        pub request_value_max: FixpDbl,
    }
}

pod_union! {
    /// `DRC_FEATURE_REQUEST` (`shouldBeUnion`, a union in the reference build)
    pub union DrcFeatureRequest [largest = DrcEffectTypeFeature] {
        pub drc_effect_type: DrcEffectTypeFeature,
        pub dynamic_range: DynamicRangeFeature,
        pub drc_characteristic: u8,
    }
}

impl DrcFeatureRequest {
    /// `drcEffectType`
    pub fn drc_effect_type(&self) -> &DrcEffectTypeFeature {
        // SAFETY: Pod union, every bit pattern is a valid member.
        unsafe { &self.drc_effect_type }
    }

    /// `drcEffectType`
    pub fn drc_effect_type_mut(&mut self) -> &mut DrcEffectTypeFeature {
        // SAFETY: Pod union, every bit pattern is a valid member.
        unsafe { &mut self.drc_effect_type }
    }

    /// `dynamicRange`
    pub fn dynamic_range(&self) -> &DynamicRangeFeature {
        // SAFETY: Pod union, every bit pattern is a valid member.
        unsafe { &self.dynamic_range }
    }

    /// `drcCharacteristic`
    pub fn drc_characteristic(&self) -> u8 {
        // SAFETY: Pod union, every bit pattern is a valid member.
        unsafe { self.drc_characteristic }
    }
}

pod_struct! {
    /// `SEL_PROC_INPUT`
    pub struct SelProcInput {
        /* system parameters */
        pub base_channel_count: i8,
        /// not supported
        pub base_layout: i8,
        pub _pad0: [u8; 2],
        pub target_config_request_type: TargetConfigRequestType,
        pub num_downmix_id_requests: u8,
        pub downmix_id_requested: [u8; MAX_REQUESTS_DOWNMIX_ID],
        pub target_layout_requested: u8,
        pub target_channel_count_requested: u8,
        pub _pad1: [u8; 2],
        /// needed for complexity estimation, currently not supported
        pub audio_sample_rate: i32,

        /* loudness normalization parameters */
        pub loudness_normalization_on: u8,
        pub _pad2: [u8; 3],
        /// e = 7
        pub target_loudness: FixpDbl,
        pub album_mode: u8,
        pub peak_limiter_present: u8,
        /// resolution: 1 dB
        pub loudness_deviation_max: u8,
        pub _pad3: u8,
        pub loudness_measurement_method: MethodDefinitionRequest,
        pub loudness_measurement_system: MeasurementSystemRequest,
        /// not supported
        pub loudness_measurement_pre_proc: LoudnessPreprocessingRequest,
        /// not supported
        pub device_cut_off_frequency: i32,
        /// e = 7
        pub loudness_normalization_gain_db_max: FixpDbl,
        /// e = 7
        pub loudness_normalization_gain_modification_db: FixpDbl,
        /// e = 7
        pub output_peak_level_max: FixpDbl,

        /* dynamic range control parameters */
        pub dynamic_range_control_on: u8,
        pub num_drc_feature_requests: u8,
        pub _pad4: [u8; 2],
        pub drc_feature_request_type: [DrcFeatureRequestType; MAX_REQUESTS_DRC_FEATURE],
        pub drc_feature_request: [DrcFeatureRequest; MAX_REQUESTS_DRC_FEATURE],

        /* other */
        /// e = 1
        pub boost: FixpSgl,
        /// e = 1
        pub compress: FixpSgl,
        /// not supported
        pub drc_characteristic_target: u8,
        pub _pad5: [u8; 3],
    }
}

/// `fallbackEffectTypeRequests`: Table E.1 of ISO/IEC DIS 23003-4: Recommended order of fallback
/// effect type requests. The values of
///
/// `scripts/dump_table.py libDRCdec/src/drcDec_selectionProcess.cpp fallbackEffectTypeRequests INT --per-line 5`
///
/// written with the names of the `DRC_EFFECT_TYPE_REQUEST` enumerators.
pub static FALLBACK_EFFECT_TYPE_REQUESTS: [[DrcEffectTypeRequest; 5]; 6] = [
    /* Night */ [DETR_GENERAL_COMPR, DETR_NOISY, DETR_LIMITED, DETR_LOWLEVEL, DETR_DIALOG],
    /* Noisy */ [DETR_GENERAL_COMPR, DETR_NIGHT, DETR_LIMITED, DETR_LOWLEVEL, DETR_DIALOG],
    /* Limited */ [DETR_GENERAL_COMPR, DETR_NIGHT, DETR_NOISY, DETR_LOWLEVEL, DETR_DIALOG],
    /* LowLevel */ [DETR_GENERAL_COMPR, DETR_NOISY, DETR_NIGHT, DETR_LIMITED, DETR_DIALOG],
    /* Dialog */ [DETR_GENERAL_COMPR, DETR_NIGHT, DETR_NOISY, DETR_LIMITED, DETR_LOWLEVEL],
    /* General */ [DETR_NIGHT, DETR_NOISY, DETR_LIMITED, DETR_LOWLEVEL, DETR_DIALOG],
];

/// The number of entries of a candidate list: `(12 + 1 + 6)`.
const SELECTION_SIZE: usize = 12 + 1 + 6;

pod_struct! {
    /// `DRCDEC_SELECTION_DATA`
    pub struct DrcdecSelectionData {
        pub selection_flag: u8,
        pub downmix_id_request_index: u8,
        pub _pad0: [u8; 2],
        /// e = 7
        pub output_peak_level: FixpDbl,
        /// e = 7
        pub loudness_normalization_gain_db_adjusted: FixpDbl,
        /// e = 7
        pub output_loudness: FixpDbl,
        /// `pInst`: index into `drcInstructionsUniDrc` of the `UNI_DRC_CONFIG` plus one, 0 for
        /// `NULL` (pointer-sized, as the C pointer)
        pub p_inst: isize,
    }
}

impl DrcdecSelectionData {
    /// `pInst` as an index into `drcInstructionsUniDrc` (never `NULL` for a candidate).
    pub fn inst(&self) -> usize {
        (self.p_inst.max(1) - 1) as usize
    }
}

pod_struct! {
    /// `DRCDEC_SELECTION`
    pub struct DrcdecSelection {
        pub num_data: u8,
        /// Up to the alignment of the pointers in `data` (8 or 4 bytes).
        pub _pad0: [u8; if cfg!(target_pointer_width = "32") { 3 } else { 7 }],
        pub data: [DrcdecSelectionData; SELECTION_SIZE],
    }
}

pod_struct! {
    /// `struct s_drcdec_selection_process`
    pub struct DrcSelectionProcess {
        pub codec_mode: SelProcCodecMode,
        pub sel_proc_input: SelProcInput,
        /// Up to the alignment of the pointers in `selection_data` (none with 4-byte pointers).
        pub _pad0: [u8; if cfg!(target_pointer_width = "32") { 0 } else { 4 }],
        /// 2 instances, one before and one after selection
        pub selection_data: [DrcdecSelection; 2],
    }
}

/// The two candidate lists (`ppCandidatesPotential`, `ppCandidatesSelected`): indices of
/// `selectionData`.
#[derive(Clone, Copy)]
struct Lists {
    pot: usize,
    sel: usize,
}

/*******************************************/
/* helper functions                        */
/*******************************************/

/// `_isError(x)`
fn is_error(x: i32) -> i32 {
    if x < DRCDEC_SELECTION_PROCESS_WARNING {
        return 1;
    }

    0
}

/// `_drcdec_selection_addNew(pSelection)`
fn selection_add_new(p_selection: &mut DrcdecSelection) -> Option<&mut DrcdecSelectionData> {
    if (p_selection.num_data as usize) < SELECTION_SIZE {
        let p_data = &mut p_selection.data[p_selection.num_data as usize];
        *p_data = DrcdecSelectionData::ZERO;
        p_selection.num_data += 1;

        Some(p_data)
    } else {
        None
    }
}

/// `_drcdec_selection_add(pSelection, pDataIn)`
fn selection_add(p_selection: &mut DrcdecSelection, p_data_in: &DrcdecSelectionData) -> bool {
    if (p_selection.num_data as usize) < SELECTION_SIZE {
        p_selection.data[p_selection.num_data as usize] = *p_data_in;
        p_selection.num_data += 1;
        true
    } else {
        false
    }
}

/// `_drcdec_selection_clear(pSelection)`
fn selection_clear(p_selection: &mut DrcdecSelection) -> i32 {
    p_selection.num_data = 0;
    0
}

/// `_drcdec_selection_getNumber(pSelection)`
fn selection_get_number(p_selection: &DrcdecSelection) -> i32 {
    p_selection.num_data as i32
}

/// `_drcdec_selection_setNumber(pSelection, num)`
fn selection_set_number(p_selection: &mut DrcdecSelection, num: i32) -> i32 {
    if num >= 0 && num < p_selection.num_data as i32 {
        p_selection.num_data = num as u8;
        num
    } else {
        p_selection.num_data as i32
    }
}

/// `_drcdec_selection_getAt(pSelection, at)` (a copy of the entry)
fn selection_get_at(p_selection: &DrcdecSelection, at: i32) -> Option<DrcdecSelectionData> {
    if at >= 0 && (at as usize) < SELECTION_SIZE {
        Some(p_selection.data[at as usize])
    } else {
        None
    }
}

/// `_swapSelectionAndClear(ppCandidatesPotential, ppCandidatesSelected)`
fn swap_selection_and_clear(sd: &mut [DrcdecSelection; 2], l: &mut Lists) -> i32 {
    core::mem::swap(&mut l.pot, &mut l.sel);
    selection_clear(&mut sd[l.sel]);
    0
}

/// `_swapSelection(ppCandidatesPotential, ppCandidatesSelected)`
fn swap_selection(l: &mut Lists) -> i32 {
    core::mem::swap(&mut l.pot, &mut l.sel);
    0
}

/*******************************************/
/* public functions                        */
/*******************************************/

/// `drcDec_SelectionProcess_Create(phInstance)`
pub fn drc_dec_selection_process_create(
    ph_instance: &mut HandleDrcSelectionProcess,
) -> DrcdecSelectionProcessReturn {
    let mut h_instance = Box::new(DrcSelectionProcess::ZERO);

    h_instance.codec_mode = SEL_PROC_CODEC_MODE_UNDEFINED;

    *ph_instance = Some(h_instance);
    DRCDEC_SELECTION_PROCESS_NO_ERROR
}

/// `drcDec_SelectionProcess_Init(hInstance)`
pub fn drc_dec_selection_process_init(
    h_instance: Option<&mut DrcSelectionProcess>,
) -> DrcdecSelectionProcessReturn {
    let Some(h_instance) = h_instance else {
        return DRCDEC_SELECTION_PROCESS_NOT_OK;
    };

    init_default_params(&mut h_instance.sel_proc_input);
    DRCDEC_SELECTION_PROCESS_NO_ERROR
}

/// `drcDec_SelectionProcess_SetCodecMode(hInstance, codecMode)`
pub fn drc_dec_selection_process_set_codec_mode(
    h_instance: Option<&mut DrcSelectionProcess>,
    codec_mode: SelProcCodecMode,
) -> DrcdecSelectionProcessReturn {
    let Some(h_instance) = h_instance else {
        return DRCDEC_SELECTION_PROCESS_NOT_OK;
    };

    match codec_mode {
        SEL_PROC_MPEG_4_AAC
        | SEL_PROC_MPEG_D_USAC
        | SEL_PROC_TEST_TIME_DOMAIN
        | SEL_PROC_TEST_QMF_DOMAIN
        | SEL_PROC_TEST_STFT_DOMAIN => {
            h_instance.codec_mode = codec_mode;
        }
        _ => return DRCDEC_SELECTION_PROCESS_NOT_OK,
    }

    h_instance.codec_mode = codec_mode;
    init_codec_mode_params(&mut h_instance.sel_proc_input, codec_mode)
}

/// `FL2FXCONST_DBL(-63.0f / (float)(1 << 7))`
const TARGET_LOUDNESS_MIN: FixpDbl = fl2fxconst_dbl((-63.0f32 / 128.0f32) as f64);
/// `FL2FXCONST_DBL(-10.0f / (float)(1 << 7))`
const TARGET_LOUDNESS_MAX_RECOMMENDED: FixpDbl = fl2fxconst_dbl((-10.0f32 / 128.0f32) as f64);

/// `drcDec_SelectionProcess_SetParam(hInstance, requestType, requestValue, pDiff)`
pub fn drc_dec_selection_process_set_param(
    h_instance: &mut DrcSelectionProcess,
    request_type: SelProcUserParam,
    mut request_value: FixpDbl,
    p_diff: Option<&mut i32>,
) -> DrcdecSelectionProcessReturn {
    let request_value_int = request_value;
    let mut diff = 0i32;
    let p_sel_proc_input = &mut h_instance.sel_proc_input;

    match request_type {
        SEL_PROC_LOUDNESS_NORMALIZATION_ON => {
            if (request_value_int != 0) && (request_value_int != 1) {
                return DRCDEC_SELECTION_PROCESS_PARAM_OUT_OF_RANGE;
            }
            diff |= comp_assign(
                &mut p_sel_proc_input.loudness_normalization_on,
                request_value_int as u8,
            );
        }
        SEL_PROC_TARGET_LOUDNESS => {
            /* Lower boundary: drcSetTargetLoudnessValueLower default value.
            Upper boundary: drcSetTargetLoudnessValueUpper default value */
            if (request_value < TARGET_LOUDNESS_MIN) || (request_value > 0) {
                return DRCDEC_SELECTION_PROCESS_PARAM_OUT_OF_RANGE;
            }
            if request_value > TARGET_LOUDNESS_MAX_RECOMMENDED {
                /* recommended maximum value */
                request_value = TARGET_LOUDNESS_MAX_RECOMMENDED;
            }
            diff |= comp_assign(&mut p_sel_proc_input.target_loudness, request_value);
        }
        SEL_PROC_EFFECT_TYPE => {
            if (request_value_int < -1) || (request_value_int >= DETR_COUNT) {
                return DRCDEC_SELECTION_PROCESS_PARAM_OUT_OF_RANGE;
            }
            /* Caution. This overrides all drcFeatureRequests requested so far! */
            if request_value_int == -1 {
                diff |= comp_assign(&mut p_sel_proc_input.dynamic_range_control_on, 0);
            } else if request_value_int == DETR_NONE {
                diff |= comp_assign(&mut p_sel_proc_input.dynamic_range_control_on, 1);
                diff |= comp_assign(&mut p_sel_proc_input.num_drc_feature_requests, 0);
            } else {
                diff |= comp_assign(&mut p_sel_proc_input.dynamic_range_control_on, 1);
                diff |= comp_assign(&mut p_sel_proc_input.num_drc_feature_requests, 1);
                diff |= comp_assign(
                    &mut p_sel_proc_input.drc_feature_request_type[0],
                    DFRT_EFFECT_TYPE,
                );
                let et = p_sel_proc_input.drc_feature_request[0].drc_effect_type_mut();
                diff |= comp_assign(&mut et.num_requests_desired, 1);
                diff |= comp_assign(&mut et.request[0], request_value_int);
                if (request_value_int > DETR_NONE) && (request_value_int <= DETR_GENERAL_COMPR) {
                    /* use fallback effect type requests */
                    for i in 0..5 {
                        diff |= comp_assign(
                            &mut et.request[i + 1],
                            FALLBACK_EFFECT_TYPE_REQUESTS[(request_value_int - 1) as usize][i],
                        );
                    }
                    diff |= comp_assign(&mut et.num_requests, 6);
                } else {
                    diff |= comp_assign(&mut et.num_requests, 1);
                }
            }
        }
        SEL_PROC_LOUDNESS_MEASUREMENT_METHOD => {
            if (request_value_int < 0) || (request_value_int > 2) {
                return DRCDEC_SELECTION_PROCESS_PARAM_OUT_OF_RANGE;
            }
            diff |=
                comp_assign(&mut p_sel_proc_input.loudness_measurement_method, request_value_int);
        }
        SEL_PROC_ALBUM_MODE => {
            if (request_value_int < 0) || (request_value_int > 1) {
                return DRCDEC_SELECTION_PROCESS_PARAM_OUT_OF_RANGE;
            }
            diff |= comp_assign(&mut p_sel_proc_input.album_mode, request_value_int as u8);
        }
        SEL_PROC_DOWNMIX_ID => {
            diff |= comp_assign(&mut p_sel_proc_input.target_config_request_type, TCRT_DOWNMIX_ID);
            if request_value_int < 0 {
                /* negative requests signal no downmixId */
                diff |= comp_assign(&mut p_sel_proc_input.num_downmix_id_requests, 0);
            } else {
                diff |= comp_assign(&mut p_sel_proc_input.num_downmix_id_requests, 1);
                diff |= comp_assign(
                    &mut p_sel_proc_input.downmix_id_requested[0],
                    request_value_int as u8,
                );
            }
        }
        SEL_PROC_TARGET_LAYOUT => {
            /* Request target layout according to ChannelConfiguration in ISO/IEC 23001-8
             * (CICP) */
            if (request_value_int < 1) || (request_value_int > 63) {
                return DRCDEC_SELECTION_PROCESS_PARAM_OUT_OF_RANGE;
            }
            diff |=
                comp_assign(&mut p_sel_proc_input.target_config_request_type, TCRT_TARGET_LAYOUT);
            diff |=
                comp_assign(&mut p_sel_proc_input.target_layout_requested, request_value_int as u8);
        }
        SEL_PROC_TARGET_CHANNEL_COUNT => {
            if (request_value_int < 1) || (request_value_int > 8) {
                return DRCDEC_SELECTION_PROCESS_PARAM_OUT_OF_RANGE;
            }
            diff |= comp_assign(
                &mut p_sel_proc_input.target_config_request_type,
                TCRT_TARGET_CHANNEL_COUNT,
            );
            diff |= comp_assign(
                &mut p_sel_proc_input.target_channel_count_requested,
                request_value_int as u8,
            );
        }
        SEL_PROC_BASE_CHANNEL_COUNT => {
            if request_value_int < 0 {
                return DRCDEC_SELECTION_PROCESS_PARAM_OUT_OF_RANGE;
            }
            diff |= comp_assign(&mut p_sel_proc_input.base_channel_count, request_value_int as i8);
        }
        SEL_PROC_SAMPLE_RATE => {
            if request_value_int < 0 {
                return DRCDEC_SELECTION_PROCESS_PARAM_OUT_OF_RANGE;
            }
            diff |= comp_assign(&mut p_sel_proc_input.audio_sample_rate, request_value_int);
        }
        SEL_PROC_BOOST => {
            if (request_value < 0) || (request_value > fl2fxconst_dbl(1.0 / 2.0)) {
                return DRCDEC_SELECTION_PROCESS_PARAM_OUT_OF_RANGE;
            }
            /* convert to FIXP_SGL with rounding */
            diff |= comp_assign(
                &mut p_sel_proc_input.boost,
                fx_dbl2fx_sgl(request_value.wrapping_add(1 << 15)),
            );
        }
        SEL_PROC_COMPRESS => {
            if (request_value < 0) || (request_value > fl2fxconst_dbl(1.0 / 2.0)) {
                return DRCDEC_SELECTION_PROCESS_PARAM_OUT_OF_RANGE;
            }
            /* convert to FIXP_SGL with rounding */
            diff |= comp_assign(
                &mut p_sel_proc_input.compress,
                fx_dbl2fx_sgl(request_value.wrapping_add(1 << 15)),
            );
        }
        _ => return DRCDEC_SELECTION_PROCESS_INVALID_PARAM,
    }

    if let Some(p_diff) = p_diff {
        *p_diff |= diff;
    }

    DRCDEC_SELECTION_PROCESS_NO_ERROR
}

/// `drcDec_SelectionProcess_GetParam(hInstance, requestType)`
pub fn drc_dec_selection_process_get_param(
    h_instance: &DrcSelectionProcess,
    request_type: SelProcUserParam,
) -> FixpDbl {
    let p_sel_proc_input = &h_instance.sel_proc_input;

    match request_type {
        SEL_PROC_LOUDNESS_NORMALIZATION_ON => p_sel_proc_input.loudness_normalization_on as FixpDbl,
        SEL_PROC_DYNAMIC_RANGE_CONTROL_ON => p_sel_proc_input.dynamic_range_control_on as FixpDbl,
        _ => 0,
    }
}

/// `drcDec_SelectionProcess_Delete(phInstance)`
pub fn drc_dec_selection_process_delete(
    ph_instance: &mut HandleDrcSelectionProcess,
) -> DrcdecSelectionProcessReturn {
    if ph_instance.is_none() {
        return DRCDEC_SELECTION_PROCESS_INVALID_HANDLE;
    }

    *ph_instance = None;
    DRCDEC_SELECTION_PROCESS_NO_ERROR
}

/// `drcDec_SelectionProcess_Process(hInstance, hUniDrcConfig, hLoudnessInfoSet,
/// hSelProcOutput)`
pub fn drc_dec_selection_process_process(
    h_instance: Option<&mut DrcSelectionProcess>,
    h_uni_drc_config: &mut UniDrcConfig,
    h_loudness_info_set: &LoudnessInfoSet,
    h_sel_proc_output: &mut SelProcOutput,
) -> DrcdecSelectionProcessReturn {
    let Some(h_instance) = h_instance else {
        return DRCDEC_SELECTION_PROCESS_INVALID_HANDLE;
    };
    let codec_mode = h_instance.codec_mode;
    let input = &mut h_instance.sel_proc_input;
    let sd = &mut h_instance.selection_data;

    let mut l = Lists { pot: 1, sel: 0 };
    selection_set_number(&mut sd[l.sel], 0);
    selection_set_number(&mut sd[l.pot], 0);

    let mut ret_val = generate_virtual_drc_sets(input, h_uni_drc_config, codec_mode);
    if ret_val != 0 {
        return ret_val;
    }

    if input.base_channel_count != h_uni_drc_config.channel_layout.base_channel_count as i8 {
        input.base_channel_count = h_uni_drc_config.channel_layout.base_channel_count as i8;
    }

    if (input.target_config_request_type != 0)
        || (input.target_config_request_type == 0 && input.num_downmix_id_requests == 0)
    {
        ret_val = channel_layout_to_downmix_id_mapping(input, h_uni_drc_config);

        if is_error(ret_val) != 0 {
            return ret_val;
        }
    }

    ret_val =
        drc_set_pre_selection(input, h_uni_drc_config, h_loudness_info_set, sd, &mut l, codec_mode);
    if ret_val != 0 {
        return ret_val;
    }

    if input.album_mode != 0 {
        swap_selection_and_clear(sd, &mut l);

        ret_val = select_album_loudness(h_uni_drc_config, h_loudness_info_set, sd, l);
        if ret_val != 0 {
            return ret_val;
        }

        if selection_get_number(&sd[l.sel]) == 0 {
            swap_selection(&mut l);
        }
    }

    swap_selection_and_clear(sd, &mut l);

    ret_val = drc_set_request_selection(input, h_uni_drc_config, h_loudness_info_set, sd, &mut l);
    if ret_val != 0 {
        return ret_val;
    }

    ret_val = drc_set_final_selection(input, h_uni_drc_config, sd, &mut l, codec_mode);
    if ret_val != 0 {
        return ret_val;
    }

    let selection_data = sd[l.sel].data[0];
    ret_val = generate_output_info(
        input,
        h_sel_proc_output,
        h_uni_drc_config,
        h_loudness_info_set,
        &selection_data,
        codec_mode,
    );

    if is_error(ret_val) != 0 {
        return ret_val;
    }

    ret_val = select_downmix_matrix(h_sel_proc_output, h_uni_drc_config);
    if ret_val != 0 {
        return ret_val;
    }

    DRCDEC_SELECTION_PROCESS_NO_ERROR
}

/*******************************************/
/* static functions                        */
/*******************************************/

/// `_initDefaultParams(hSelProcInput)`
fn init_default_params(h_sel_proc_input: &mut SelProcInput) -> DrcdecSelectionProcessReturn {
    let ret_val = DRCDEC_SELECTION_PROCESS_NO_ERROR;

    /* system parameters */
    h_sel_proc_input.base_channel_count = -1;
    h_sel_proc_input.base_layout = -1;
    h_sel_proc_input.target_config_request_type = TCRT_DOWNMIX_ID;
    h_sel_proc_input.num_downmix_id_requests = 0;

    /* loudness normalization parameters */
    h_sel_proc_input.album_mode = 0;
    h_sel_proc_input.peak_limiter_present = 0;
    h_sel_proc_input.loudness_normalization_on = 1;
    h_sel_proc_input.target_loudness = fl2fxconst_dbl((-24.0f32 / 128.0f32) as f64);
    h_sel_proc_input.loudness_deviation_max = DEFAULT_LOUDNESS_DEVIATION_MAX;
    h_sel_proc_input.loudness_measurement_method = MDR_ANCHOR_LOUDNESS;
    h_sel_proc_input.loudness_measurement_system = MSR_EXPERT_PANEL;
    h_sel_proc_input.loudness_measurement_pre_proc = LPR_DEFAULT;
    h_sel_proc_input.device_cut_off_frequency = 500;
    h_sel_proc_input.loudness_normalization_gain_db_max = MAXVAL_DBL; /* infinity as default */
    h_sel_proc_input.loudness_normalization_gain_modification_db = 0;
    h_sel_proc_input.output_peak_level_max = 0;
    if h_sel_proc_input.peak_limiter_present == 1 {
        h_sel_proc_input.output_peak_level_max = fl2fxconst_dbl((6.0f32 / 128.0f32) as f64);
    }

    /* dynamic range control parameters */
    h_sel_proc_input.dynamic_range_control_on = 1;

    h_sel_proc_input.num_drc_feature_requests = 0;

    /* other parameters */
    h_sel_proc_input.boost = fl2fxconst_sgl((1.0f32 / 2.0f32) as f64);
    h_sel_proc_input.compress = fl2fxconst_sgl((1.0f32 / 2.0f32) as f64);
    h_sel_proc_input.drc_characteristic_target = 0;

    ret_val
}

/// `_initCodecModeParams(hSelProcInput, codecMode)`
fn init_codec_mode_params(
    h_sel_proc_input: &mut SelProcInput,
    codec_mode: SelProcCodecMode,
) -> DrcdecSelectionProcessReturn {
    let ret_val = DRCDEC_SELECTION_PROCESS_NO_ERROR;

    match codec_mode {
        SEL_PROC_MPEG_H_3DA => {
            h_sel_proc_input.loudness_deviation_max = 0;
            h_sel_proc_input.peak_limiter_present = 1; /* peak limiter is mandatory */
            /* The peak limiter also has to catch overshoots due to user interactivity,
            downmixing etc. Therefore the maximum output peak level is reduced to 0 dB. */
            h_sel_proc_input.output_peak_level_max = 0;
        }
        SEL_PROC_MPEG_4_AAC | SEL_PROC_MPEG_D_USAC => {
            h_sel_proc_input.loudness_deviation_max = DEFAULT_LOUDNESS_DEVIATION_MAX;
            h_sel_proc_input.peak_limiter_present = 1;
            /* A peak limiter is present at the end of the decoder, therefore we can allow for a
             * maximum output peak level greater than full scale */
            h_sel_proc_input.output_peak_level_max = fl2fxconst_dbl((6.0f32 / 128.0f32) as f64);
        }
        SEL_PROC_TEST_TIME_DOMAIN | SEL_PROC_TEST_QMF_DOMAIN | SEL_PROC_TEST_STFT_DOMAIN => {
            /* for testing, adapt to default settings in reference software */
            h_sel_proc_input.loudness_normalization_on = 0;
            h_sel_proc_input.dynamic_range_control_on = 0;
        }
        _ => {
            h_sel_proc_input.loudness_deviation_max = DEFAULT_LOUDNESS_DEVIATION_MAX;
            h_sel_proc_input.peak_limiter_present = 0;
        }
    }

    ret_val
}

/// `_channelLayoutToDownmixIdMapping(hSelProcInput, hUniDrcConfig)`
fn channel_layout_to_downmix_id_mapping(
    h_sel_proc_input: &mut SelProcInput,
    h_uni_drc_config: &UniDrcConfig,
) -> DrcdecSelectionProcessReturn {
    let mut ret_val = DRCDEC_SELECTION_PROCESS_NO_ERROR;

    h_sel_proc_input.num_downmix_id_requests = 0;

    match h_sel_proc_input.target_config_request_type {
        TCRT_DOWNMIX_ID => {
            if h_sel_proc_input.num_downmix_id_requests == 0 {
                h_sel_proc_input.downmix_id_requested[0] = 0;
                h_sel_proc_input.num_downmix_id_requests = 1;
            }
        }

        TCRT_TARGET_LAYOUT => {
            if h_sel_proc_input.target_layout_requested as i32
                == h_sel_proc_input.base_layout as i32
            {
                h_sel_proc_input.downmix_id_requested[0] = 0;
                h_sel_proc_input.num_downmix_id_requests = 1;
            }

            if h_sel_proc_input.num_downmix_id_requests == 0 {
                for i in 0..h_uni_drc_config.downmix_instructions_count.min(6) as usize {
                    let p_down = &h_uni_drc_config.downmix_instructions[i];

                    if h_sel_proc_input.target_layout_requested == p_down.target_layout {
                        let n = h_sel_proc_input.num_downmix_id_requests as usize;
                        h_sel_proc_input.downmix_id_requested[n] = p_down.downmix_id;
                        h_sel_proc_input.num_downmix_id_requests += 1;
                    }
                }
            }

            if h_sel_proc_input.base_layout == -1 {
                ret_val = DRCDEC_SELECTION_PROCESS_WARNING;
            }

            if h_sel_proc_input.num_downmix_id_requests == 0 {
                h_sel_proc_input.downmix_id_requested[0] = 0;
                h_sel_proc_input.num_downmix_id_requests = 1;
                ret_val = DRCDEC_SELECTION_PROCESS_WARNING;
            }
        }

        TCRT_TARGET_CHANNEL_COUNT => {
            if h_sel_proc_input.target_channel_count_requested as i32
                == h_sel_proc_input.base_channel_count as i32
            {
                h_sel_proc_input.downmix_id_requested[0] = 0;
                h_sel_proc_input.num_downmix_id_requests = 1;
            }

            if h_sel_proc_input.num_downmix_id_requests == 0 {
                for i in 0..h_uni_drc_config.downmix_instructions_count.min(6) as usize {
                    let p_down = &h_uni_drc_config.downmix_instructions[i];

                    if h_sel_proc_input.target_channel_count_requested
                        == p_down.target_channel_count
                    {
                        let n = h_sel_proc_input.num_downmix_id_requests as usize;
                        h_sel_proc_input.downmix_id_requested[n] = p_down.downmix_id;
                        h_sel_proc_input.num_downmix_id_requests += 1;
                    }
                }
            }

            if h_sel_proc_input.base_channel_count == -1 {
                ret_val = DRCDEC_SELECTION_PROCESS_WARNING;
            }

            if h_sel_proc_input.num_downmix_id_requests == 0 {
                ret_val = DRCDEC_SELECTION_PROCESS_WARNING;
                h_sel_proc_input.downmix_id_requested[0] = 0;
                h_sel_proc_input.num_downmix_id_requests = 1;
            }
        }

        _ => return DRCDEC_SELECTION_PROCESS_NOT_OK,
    }

    ret_val
}

/*******************************************/

/* Note: Numbering of DRC pre-selection steps according to MPEG-D Part-4 DRC Amd1 */

/// `_preSelectionRequirement123(nRequestedDownmixId, pDrcInstructionUniDrc, pMatchFound)`
///
/// #1: DownmixId of DRC set matches the requested downmixId.
/// #2: Output channel layout of DRC set matches the requested layout.
/// #3: Channel count of DRC set matches the requested channel count.
fn pre_selection_requirement123(
    n_requested_downmix_id: i32,
    p_drc_instruction_uni_drc: &DrcInstructionsUniDrc,
    p_match_found: &mut i32,
) -> DrcdecSelectionProcessReturn {
    *p_match_found = 0;

    // downmixIdCount is at most 8 (reader).
    for i in 0..(p_drc_instruction_uni_drc.downmix_id_count as usize).min(8) {
        let id = p_drc_instruction_uni_drc.downmix_id[i] as i32;
        if (id == n_requested_downmix_id)
            || (id == DOWNMIX_ID_ANY_DOWNMIX)
            || ((id == DOWNMIX_ID_BASE_LAYOUT) && (p_drc_instruction_uni_drc.drc_set_id > 0))
        {
            *p_match_found = 1;
            break;
        }
    }

    DRCDEC_SELECTION_PROCESS_NO_ERROR
}

/// `_preSelectionRequirement4(pDrcInstruction, nDynamicRangeControlOn, pMatchFound)`
///
/// #4: The DRC set is not a "Fade-" or "Ducking-" only DRC set.
fn pre_selection_requirement4(
    p_drc_instruction: &DrcInstructionsUniDrc,
    n_dynamic_range_control_on: i32,
    p_match_found: &mut i32,
) -> DrcdecSelectionProcessReturn {
    *p_match_found = 0;

    if n_dynamic_range_control_on == 1 {
        let effect = p_drc_instruction.drc_set_effect as i32;
        if (effect != EB_FADE)
            && (effect != EB_DUCK_OTHER)
            && (effect != EB_DUCK_SELF)
            && (effect != 0 || p_drc_instruction.drc_set_id < 0)
        {
            *p_match_found = 1;
        }
    } else {
        *p_match_found = 1;
    }

    DRCDEC_SELECTION_PROCESS_NO_ERROR
}

/// `_preSelectionRequirement5(pDrcInstructionUniDrc, pCoef, pMatchFound)`
///
/// #5: The number of DRC bands is supported. Moreover, gainSetIndex and gainSequenceIndex are
/// within the allowed range.
fn pre_selection_requirement5(
    p_drc_instruction_uni_drc: &DrcInstructionsUniDrc,
    p_coef: Option<&DrcCoefficientsUniDrc>,
    p_match_found: &mut i32,
) -> DrcdecSelectionProcessReturn {
    *p_match_found = 1;

    if p_drc_instruction_uni_drc.drc_set_id < 0 {
        /* virtual DRC sets are okay */
        return DRCDEC_SELECTION_PROCESS_NO_ERROR;
    }

    let Some(p_coef) = p_coef else {
        /* check for parametricDRC */
        *p_match_found = 0; /* parametricDRC not supported */
        return DRCDEC_SELECTION_PROCESS_NO_ERROR;
    };

    if p_coef.drc_location != p_drc_instruction_uni_drc.drc_location {
        /* drcLocation must be LOCATION_SELECTED */
        *p_match_found = 0;
        return DRCDEC_SELECTION_PROCESS_NO_ERROR;
    }

    for i in 0..(p_drc_instruction_uni_drc.n_drc_channel_groups as usize).min(8) {
        let index_drc_coeff = p_drc_instruction_uni_drc.gain_set_index_for_channel_group[i] as i32;

        if index_drc_coeff >= 12 {
            *p_match_found = 0;
            return DRCDEC_SELECTION_PROCESS_NO_ERROR;
        }

        if index_drc_coeff > p_coef.gain_set_count as i32 - 1 {
            /* check for parametricDRC */
            continue;
        }

        // indexDrcCoeff >= 0: the channel groups have gain set indices >= 0.
        let gain_set = &p_coef.gain_set[(index_drc_coeff.max(0)) as usize];
        let band_count = gain_set.band_count as usize;

        if band_count > 4 {
            *p_match_found = 0;
        }

        for b in 0..band_count {
            // gainSequenceIndex[b] for b >= 4 are the following bytes of the GAIN_SET,
            // reproduced from the C layout.
            let gsi =
                pod_read_u8(gain_set, core::mem::offset_of!(GainSet, gain_sequence_index) + b)
                    .unwrap_or(0);
            if (gsi >= 12) || (gsi >= p_coef.gain_sequence_count) {
                *p_match_found = 0;
                return DRCDEC_SELECTION_PROCESS_NO_ERROR;
            }
        }
    }

    DRCDEC_SELECTION_PROCESS_NO_ERROR
}

/// `_preSelectionRequirement6(pDrcInstructionUniDrc, pMatchFound)`
///
/// #6: Independent use of DRC set is permitted.
fn pre_selection_requirement6(
    p_drc_instruction_uni_drc: &DrcInstructionsUniDrc,
    p_match_found: &mut i32,
) -> DrcdecSelectionProcessReturn {
    *p_match_found = 0;

    if ((p_drc_instruction_uni_drc.depends_on_drc_set_present == 0)
        && (p_drc_instruction_uni_drc.no_independent_use() == 0))
        || (p_drc_instruction_uni_drc.depends_on_drc_set_present == 1)
    {
        *p_match_found = 1;
    }

    DRCDEC_SELECTION_PROCESS_NO_ERROR
}

/// `_preSelectionRequirement7(pDrcInstructionUniDrc, pMatchFound)`
///
/// #7: DRC sets that require EQ are only permitted if EQ is supported.
fn pre_selection_requirement7(
    p_drc_instruction_uni_drc: &DrcInstructionsUniDrc,
    p_match_found: &mut i32,
) -> DrcdecSelectionProcessReturn {
    *p_match_found = 1;

    if p_drc_instruction_uni_drc.requires_eq != 0 {
        /* EQ is not supported */
        *p_match_found = 0;
    }

    DRCDEC_SELECTION_PROCESS_NO_ERROR
}

/// `_setSelectionDataInfo(pData, loudness, loudnessNormalizationGainDb,
/// loudnessNormalizationGainDbMax, loudnessDeviationMax, signalPeakLevel, outputPeakLevelMax,
/// applyAdjustment)`
fn set_selection_data_info(
    p_data: &mut DrcdecSelectionData,
    mut loudness: FixpDbl,                           /* e = 7 */
    mut loudness_normalization_gain_db: FixpDbl,     /* e = 7 */
    mut loudness_normalization_gain_db_max: FixpDbl, /* e = 7 */
    mut loudness_deviation_max: FixpDbl,             /* e = 7 */
    mut signal_peak_level: FixpDbl,                  /* e = 7 */
    mut output_peak_level_max: FixpDbl,              /* e = 7 */
    apply_adjustment: i32,
) {
    let mut adjustment: FixpDbl = 0; /* e = 8 */

    /* use e = 8 for all function parameters to prevent overflow */
    loudness >>= 1;
    loudness_normalization_gain_db >>= 1;
    loudness_normalization_gain_db_max >>= 1;
    loudness_deviation_max >>= 1;
    signal_peak_level >>= 1;
    output_peak_level_max >>= 1;

    if apply_adjustment != 0 {
        adjustment = f_max(
            0,
            signal_peak_level
                .wrapping_add(loudness_normalization_gain_db)
                .wrapping_sub(output_peak_level_max),
        );
        adjustment = f_min(adjustment, f_max(0, loudness_deviation_max));
    }

    p_data.loudness_normalization_gain_db_adjusted = f_min(
        loudness_normalization_gain_db.wrapping_sub(adjustment),
        loudness_normalization_gain_db_max,
    );
    p_data.output_loudness = loudness.wrapping_add(p_data.loudness_normalization_gain_db_adjusted);
    p_data.output_peak_level =
        signal_peak_level.wrapping_add(p_data.loudness_normalization_gain_db_adjusted);

    /* shift back to e = 7 using saturation */
    p_data.loudness_normalization_gain_db_adjusted =
        saturate_left_shift(p_data.loudness_normalization_gain_db_adjusted, 1, DFRACT_BITS);
    p_data.output_loudness = saturate_left_shift(p_data.output_loudness, 1, DFRACT_BITS);
    p_data.output_peak_level = saturate_left_shift(p_data.output_peak_level, 1, DFRACT_BITS);
}

/// `_targetLoudnessInRange(pDrcInstructionUniDrc, targetLoudness)`
fn target_loudness_in_range(
    p_drc_instruction_uni_drc: &DrcInstructionsUniDrc,
    target_loudness: FixpDbl,
) -> i32 {
    let mut ret_val = 0;

    let upper = (p_drc_instruction_uni_drc.drc_set_target_loudness_value_upper as FixpDbl)
        << (DFRACT_BITS - 1 - 7);
    let lower = (p_drc_instruction_uni_drc.drc_set_target_loudness_value_lower as FixpDbl)
        << (DFRACT_BITS - 1 - 7);

    if p_drc_instruction_uni_drc.drc_set_target_loudness_present != 0
        && upper >= target_loudness
        && lower < target_loudness
    {
        ret_val = 1;
    }

    ret_val
}

/// `_drcSetIsUsable(hUniDrcConfig, pInst)`
fn drc_set_is_usable(h_uni_drc_config: &UniDrcConfig, inst: usize) -> i32 {
    let mut usable = 0;
    let p_coef = select_drc_coefficients(h_uni_drc_config, LOCATION_SELECTED)
        .map(|c| &h_uni_drc_config.drc_coefficients_uni_drc[c]);
    let p_inst = &h_uni_drc_config.drc_instructions_uni_drc[inst];

    /* check if ID is unique */
    if select_drc_instructions(h_uni_drc_config, p_inst.drc_set_id as i32) != Some(inst) {
        return 0;
    }
    /* sanity check on drcInstructions */
    pre_selection_requirement5(p_inst, p_coef, &mut usable);
    usable
}

/// `_preSelectionRequirement8(...)`
///
/// #8: The range of the target loudness specified for a DRC set has to include the requested
/// decoder target loudness.
fn pre_selection_requirement8(
    h_sel_proc_input: &SelProcInput,
    downmix_id_index: usize,
    h_uni_drc_config: &UniDrcConfig,
    h_loudness_info_set: &LoudnessInfoSet,
    inst: usize,
    sd: &mut [DrcdecSelection; 2],
    l: Lists,
    codec_mode: SelProcCodecMode,
) -> DrcdecSelectionProcessReturn {
    let p_drc_instruction_uni_drc = &h_uni_drc_config.drc_instructions_uni_drc[inst];
    let mut explicit_peak_information_present = 0;
    let mut signal_peak_level: FixpDbl = 0;
    let mut add_to_candidate = 0;

    let mut loudness_normalization_gain_db: FixpDbl = 0;
    let mut loudness: FixpDbl = 0;

    let loudness_deviation_max =
        (h_sel_proc_input.loudness_deviation_max as FixpDbl) << (DFRACT_BITS - 1 - 7);

    {
        let ret_val = get_loudness(
            h_loudness_info_set,
            h_sel_proc_input.album_mode as i32,
            h_sel_proc_input.loudness_measurement_method,
            h_sel_proc_input.loudness_measurement_system,
            h_sel_proc_input.target_loudness,
            p_drc_instruction_uni_drc.drc_set_id as i32,
            h_sel_proc_input.downmix_id_requested[downmix_id_index] as i32,
            &mut loudness_normalization_gain_db,
            &mut loudness,
        );
        if ret_val != 0 {
            return ret_val;
        }
    }

    if h_sel_proc_input.loudness_normalization_on == 0 {
        loudness_normalization_gain_db = 0;
    }

    let ret_val = get_signal_peak_level(
        h_sel_proc_input,
        h_uni_drc_config,
        h_loudness_info_set,
        p_drc_instruction_uni_drc,
        h_sel_proc_input.downmix_id_requested[downmix_id_index] as i32,
        &mut explicit_peak_information_present,
        &mut signal_peak_level,
        codec_mode,
    );
    if ret_val != 0 {
        return ret_val;
    }

    if h_sel_proc_input.dynamic_range_control_on != 0 {
        if explicit_peak_information_present == 0 {
            if p_drc_instruction_uni_drc.drc_set_target_loudness_present != 0
                && ((h_sel_proc_input.loudness_normalization_on != 0
                    && target_loudness_in_range(
                        p_drc_instruction_uni_drc,
                        h_sel_proc_input.target_loudness,
                    ) != 0)
                    || h_sel_proc_input.loudness_normalization_on == 0)
            {
                let Some(p_data) = selection_add_new(&mut sd[l.sel]) else {
                    return DRCDEC_SELECTION_PROCESS_NOT_OK;
                };

                set_selection_data_info(
                    p_data,
                    loudness,
                    loudness_normalization_gain_db,
                    h_sel_proc_input.loudness_normalization_gain_db_max,
                    loudness_deviation_max,
                    signal_peak_level,
                    h_sel_proc_input.output_peak_level_max,
                    0,
                );
                p_data.downmix_id_request_index = downmix_id_index as u8;
                p_data.p_inst = inst as isize + 1;
                /* signal pre-selection step dealing with drcSetTargetLoudness */
                p_data.selection_flag = 1;

                if h_sel_proc_input.loudness_normalization_on != 0 {
                    p_data.output_peak_level = h_sel_proc_input.target_loudness.wrapping_sub(
                        (p_drc_instruction_uni_drc.drc_set_target_loudness_value_upper as FixpDbl)
                            << (DFRACT_BITS - 1 - 7),
                    );
                } else {
                    p_data.output_peak_level = 0;
                }
            } else {
                if (h_sel_proc_input.loudness_normalization_on == 0)
                    || (p_drc_instruction_uni_drc.drc_set_target_loudness_present == 0)
                    || (h_sel_proc_input.loudness_normalization_on != 0
                        && target_loudness_in_range(
                            p_drc_instruction_uni_drc,
                            h_sel_proc_input.target_loudness,
                        ) != 0)
                {
                    add_to_candidate = 1;
                }
            }
        } else {
            add_to_candidate = 1;
        }

        if add_to_candidate != 0 {
            let Some(p_data) = selection_add_new(&mut sd[l.pot]) else {
                return DRCDEC_SELECTION_PROCESS_NOT_OK;
            };

            set_selection_data_info(
                p_data,
                loudness,
                loudness_normalization_gain_db,
                h_sel_proc_input.loudness_normalization_gain_db_max,
                loudness_deviation_max,
                signal_peak_level,
                h_sel_proc_input.output_peak_level_max,
                0,
            );
            p_data.downmix_id_request_index = downmix_id_index as u8;
            p_data.p_inst = inst as isize + 1;
            p_data.selection_flag = 0;
        }
    } else {
        if p_drc_instruction_uni_drc.drc_set_id < 0 {
            let Some(p_data) = selection_add_new(&mut sd[l.sel]) else {
                return DRCDEC_SELECTION_PROCESS_NOT_OK;
            };

            set_selection_data_info(
                p_data,
                loudness,
                loudness_normalization_gain_db,
                h_sel_proc_input.loudness_normalization_gain_db_max,
                loudness_deviation_max,
                signal_peak_level,
                h_sel_proc_input.output_peak_level_max,
                1,
            );

            p_data.downmix_id_request_index = downmix_id_index as u8;
            p_data.p_inst = inst as isize + 1;
            p_data.selection_flag = 0;
        }
    }

    DRCDEC_SELECTION_PROCESS_NO_ERROR
}

/// `_preSelectionRequirement9(hSelProcInput, pCandidatesPotential, pCandidatesSelected)`
///
/// #9: Clipping is minimized.
fn pre_selection_requirement9(
    h_sel_proc_input: &SelProcInput,
    sd: &mut [DrcdecSelection; 2],
    l: Lists,
) -> DrcdecSelectionProcessReturn {
    let mut i = 0;
    while i < selection_get_number(&sd[l.pot]) {
        let Some(p_candidate) = selection_get_at(&sd[l.pot], i) else {
            return DRCDEC_SELECTION_PROCESS_NOT_OK;
        };

        if p_candidate.output_peak_level <= h_sel_proc_input.output_peak_level_max {
            if !selection_add(&mut sd[l.sel], &p_candidate) {
                return DRCDEC_SELECTION_PROCESS_NOT_OK;
            }
        }
        i += 1;
    }

    DRCDEC_SELECTION_PROCESS_NO_ERROR
}

/// `_drcSetPreSelectionSingleInstruction(...)`
fn drc_set_pre_selection_single_instruction(
    h_sel_proc_input: &SelProcInput,
    downmix_id_index: usize,
    h_uni_drc_config: &UniDrcConfig,
    h_loudness_info_set: &LoudnessInfoSet,
    inst: usize,
    sd: &mut [DrcdecSelection; 2],
    l: Lists,
    codec_mode: SelProcCodecMode,
) -> DrcdecSelectionProcessReturn {
    let mut ret_val;
    let mut match_found = 0;
    let p_coef = select_drc_coefficients(h_uni_drc_config, LOCATION_SELECTED)
        .map(|c| &h_uni_drc_config.drc_coefficients_uni_drc[c]);
    let p_drc_instruction_uni_drc = &h_uni_drc_config.drc_instructions_uni_drc[inst];

    ret_val = pre_selection_requirement123(
        h_sel_proc_input.downmix_id_requested[downmix_id_index] as i32,
        p_drc_instruction_uni_drc,
        &mut match_found,
    );

    if ret_val == 0 && match_found != 0 {
        ret_val = pre_selection_requirement4(
            p_drc_instruction_uni_drc,
            h_sel_proc_input.dynamic_range_control_on as i32,
            &mut match_found,
        );
    }

    if ret_val == 0 && match_found != 0 {
        ret_val = pre_selection_requirement5(p_drc_instruction_uni_drc, p_coef, &mut match_found);
    }

    if ret_val == 0 && match_found != 0 {
        ret_val = pre_selection_requirement6(p_drc_instruction_uni_drc, &mut match_found);
    }

    if ret_val == 0 && match_found != 0 {
        ret_val = pre_selection_requirement7(p_drc_instruction_uni_drc, &mut match_found);
    }

    if ret_val == 0 && match_found != 0 {
        ret_val = pre_selection_requirement8(
            h_sel_proc_input,
            downmix_id_index,
            h_uni_drc_config,
            h_loudness_info_set,
            inst,
            sd,
            l,
            codec_mode,
        );
    }

    ret_val
}

/// `_drcSetSelectionAddCandidates(hSelProcInput, pCandidatesPotential, pCandidatesSelected)`
fn drc_set_selection_add_candidates(
    h_sel_proc_input: &SelProcInput,
    h_uni_drc_config: &UniDrcConfig,
    sd: &mut [DrcdecSelection; 2],
    l: Lists,
) -> DrcdecSelectionProcessReturn {
    let ret_val = DRCDEC_SELECTION_PROCESS_NO_ERROR;
    let mut n_hit_count = 0;

    for i in 0..selection_get_number(&sd[l.pot]) {
        let Some(p_candidate) = selection_get_at(&sd[l.pot], i) else {
            return DRCDEC_SELECTION_PROCESS_NOT_OK;
        };

        let p_drc_instruction_uni_drc =
            &h_uni_drc_config.drc_instructions_uni_drc[p_candidate.inst()];

        if target_loudness_in_range(p_drc_instruction_uni_drc, h_sel_proc_input.target_loudness)
            != 0
        {
            n_hit_count += 1;
        }
    }

    if n_hit_count != 0 {
        for i in 0..selection_get_number(&sd[l.pot]) {
            let Some(p_candidate) = selection_get_at(&sd[l.pot], i) else {
                return DRCDEC_SELECTION_PROCESS_NOT_OK;
            };

            let p_drc_instruction_uni_drc =
                &h_uni_drc_config.drc_instructions_uni_drc[p_candidate.inst()];

            if target_loudness_in_range(p_drc_instruction_uni_drc, h_sel_proc_input.target_loudness)
                != 0
            {
                if !selection_add(&mut sd[l.sel], &p_candidate) {
                    return DRCDEC_SELECTION_PROCESS_NOT_OK;
                }
            }
        }
    } else {
        let mut lowest_peak_level: FixpDbl = MAXVAL_DBL; /* e = 7 */

        for i in 0..selection_get_number(&sd[l.pot]) {
            let Some(p_candidate) = selection_get_at(&sd[l.pot], i) else {
                return DRCDEC_SELECTION_PROCESS_NOT_OK;
            };

            let peak_level = p_candidate.output_peak_level;

            if peak_level < lowest_peak_level {
                lowest_peak_level = peak_level;
            }
        }

        /* add all with lowest peak level or max 1dB above */
        for i in 0..selection_get_number(&sd[l.pot]) {
            let loudness_deviation_max: FixpDbl =
                (h_sel_proc_input.loudness_deviation_max as FixpDbl) << (DFRACT_BITS - 1 - 7); /* e = 7 */

            if selection_get_at(&sd[l.pot], i).is_none() {
                return DRCDEC_SELECTION_PROCESS_NOT_OK;
            }
            let p_candidate = &mut sd[l.pot].data[i as usize];

            let peak_level = p_candidate.output_peak_level;

            // lowestPeakLevel + 1 dB can overflow for a peak level near MAXVAL_DBL (undefined,
            // wraps in both builds).
            if peak_level == lowest_peak_level
                || peak_level <= lowest_peak_level.wrapping_add(fl2fxconst_dbl(1.0 / 128.0))
            {
                let mut adjustment =
                    f_max(0, peak_level.wrapping_sub(h_sel_proc_input.output_peak_level_max));
                adjustment = f_min(adjustment, f_max(0, loudness_deviation_max));

                p_candidate.loudness_normalization_gain_db_adjusted =
                    p_candidate.loudness_normalization_gain_db_adjusted.wrapping_sub(adjustment);
                p_candidate.output_peak_level =
                    p_candidate.output_peak_level.wrapping_sub(adjustment);
                p_candidate.output_loudness = p_candidate.output_loudness.wrapping_sub(adjustment);
                let c = *p_candidate;
                if !selection_add(&mut sd[l.sel], &c) {
                    return DRCDEC_SELECTION_PROCESS_NOT_OK;
                }
            }
        }
    }

    ret_val
}

/// `_dependentDrcInstruction(hUniDrcConfig, pInst, ppDrcInstructionsDependent)`
fn dependent_drc_instruction(
    h_uni_drc_config: &UniDrcConfig,
    p_inst: &DrcInstructionsUniDrc,
    pp_drc_instructions_dependent: &mut usize,
) -> DrcdecSelectionProcessReturn {
    let count = h_uni_drc_config.drc_instructions_uni_drc_count as usize;
    let mut p_dependent_drc: Option<usize> = None;
    let mut i = 0;

    while i < count {
        p_dependent_drc = Some(i);

        if h_uni_drc_config.drc_instructions_uni_drc[i].drc_set_id == p_inst.depends_on_drc_set {
            break;
        }
        i += 1;
    }

    if i == count {
        return DRCDEC_SELECTION_PROCESS_NOT_OK;
    }

    let Some(dep) = p_dependent_drc else {
        return DRCDEC_SELECTION_PROCESS_NOT_OK;
    };
    if h_uni_drc_config.drc_instructions_uni_drc[dep].depends_on_drc_set_present == 1 {
        return DRCDEC_SELECTION_PROCESS_NOT_OK;
    }

    *pp_drc_instructions_dependent = dep;

    DRCDEC_SELECTION_PROCESS_NO_ERROR
}

/// `_selectDrcSetEffectNone(hUniDrcConfig, pCandidatesPotential, pCandidatesSelected)`
fn select_drc_set_effect_none(
    h_uni_drc_config: &UniDrcConfig,
    sd: &mut [DrcdecSelection; 2],
    l: Lists,
) -> DrcdecSelectionProcessReturn {
    for i in 0..selection_get_number(&sd[l.pot]) {
        let Some(p_candidate) = selection_get_at(&sd[l.pot], i) else {
            return DRCDEC_SELECTION_PROCESS_NOT_OK;
        };

        if (h_uni_drc_config.drc_instructions_uni_drc[p_candidate.inst()].drc_set_effect & 0xff)
            == 0
        {
            if !selection_add(&mut sd[l.sel], &p_candidate) {
                return DRCDEC_SELECTION_PROCESS_NOT_OK;
            }
        }
    }

    DRCDEC_SELECTION_PROCESS_NO_ERROR
}

/// `_selectSingleEffectType(hUniDrcConfig, effectType, pCandidatesPotential,
/// pCandidatesSelected)`
fn select_single_effect_type(
    h_uni_drc_config: &UniDrcConfig,
    effect_type: DrcEffectTypeRequest,
    sd: &mut [DrcdecSelection; 2],
    l: Lists,
) -> DrcdecSelectionProcessReturn {
    let mut ret_val = DRCDEC_SELECTION_PROCESS_NO_ERROR;
    let mut p_drc_instructions_dependent = 0usize;

    if effect_type == DETR_NONE {
        ret_val = select_drc_set_effect_none(h_uni_drc_config, sd, l);
        if ret_val != 0 {
            return ret_val;
        }
    } else {
        let effect_bit_position = 1i32.wrapping_shl((effect_type - 1) as u32);

        for i in 0..selection_get_number(&sd[l.pot]) {
            let Some(p_candidate) = selection_get_at(&sd[l.pot], i) else {
                return DRCDEC_SELECTION_PROCESS_NOT_OK;
            };

            let p_inst = &h_uni_drc_config.drc_instructions_uni_drc[p_candidate.inst()];

            if p_inst.depends_on_drc_set_present == 0 {
                if (p_inst.drc_set_effect as i32 & effect_bit_position) != 0 {
                    if !selection_add(&mut sd[l.sel], &p_candidate) {
                        return DRCDEC_SELECTION_PROCESS_NOT_OK;
                    }
                }
            } else {
                ret_val = dependent_drc_instruction(
                    h_uni_drc_config,
                    p_inst,
                    &mut p_drc_instructions_dependent,
                );
                if ret_val != 0 {
                    return ret_val;
                }

                let dep = &h_uni_drc_config.drc_instructions_uni_drc[p_drc_instructions_dependent];
                if ((p_inst.drc_set_effect as i32 & effect_bit_position) != 0)
                    || ((dep.drc_set_effect as i32 & effect_bit_position) != 0)
                {
                    if !selection_add(&mut sd[l.sel], &p_candidate) {
                        return DRCDEC_SELECTION_PROCESS_NOT_OK;
                    }
                }
            }
        }
    }

    ret_val
}

/// `_selectEffectTypeFeature(hUniDrcConfig, drcFeatureRequest, ppCandidatesPotential,
/// ppCandidatesSelected)`
fn select_effect_type_feature(
    h_uni_drc_config: &UniDrcConfig,
    drc_feature_request: DrcFeatureRequest,
    sd: &mut [DrcdecSelection; 2],
    l: &mut Lists,
) -> DrcdecSelectionProcessReturn {
    let mut ret_val = DRCDEC_SELECTION_PROCESS_NO_ERROR;
    let mut desired_effect_type_found = 0;
    let et = *drc_feature_request.drc_effect_type();

    for i in 0..(et.num_requests_desired as usize).min(MAX_REQUESTS_DRC_EFFECT_TYPE) {
        ret_val = select_single_effect_type(h_uni_drc_config, et.request[i], sd, *l);
        if ret_val != 0 {
            return ret_val;
        }

        if selection_get_number(&sd[l.sel]) != 0 {
            desired_effect_type_found = 1;
            swap_selection_and_clear(sd, l);
        }
    }

    if desired_effect_type_found == 0 {
        for i in et.num_requests_desired as usize
            ..(et.num_requests as usize).min(MAX_REQUESTS_DRC_EFFECT_TYPE)
        {
            ret_val = select_single_effect_type(h_uni_drc_config, et.request[i], sd, *l);
            if ret_val != 0 {
                return ret_val;
            }

            if selection_get_number(&sd[l.sel]) != 0 {
                swap_selection_and_clear(sd, l);
                break;
            }
        }
    }

    swap_selection(l);

    ret_val
}

/// `_selectDynamicRange(hUniDrcConfig, hLoudnessInfoSet, drcFeatureRequest,
/// pDownmixIdRequested, albumMode, pCandidatesPotential, ppCandidatesSelected)`
fn select_dynamic_range(
    h_uni_drc_config: &UniDrcConfig,
    h_loudness_info_set: &LoudnessInfoSet,
    drc_feature_request: DrcFeatureRequest,
    p_downmix_id_requested: &[u8; MAX_REQUESTS_DOWNMIX_ID],
    album_mode: i32,
    sd: &mut [DrcdecSelection; 2],
    l: Lists,
) -> DrcdecSelectionProcessReturn {
    let mut ret_val = DRCDEC_SELECTION_PROCESS_NO_ERROR;
    let mut peak_to_average_present = 0;
    let mut peak_to_average: FixpDbl = 0;
    let dr = *drc_feature_request.dynamic_range();

    let mut min_val: FixpDbl = MAXVAL_DBL;

    let num_selected_candidates = selection_get_number(&sd[l.sel]);

    for i in 0..selection_get_number(&sd[l.pot]) {
        let Some(p_candidate) = selection_get_at(&sd[l.pot], i) else {
            return DRCDEC_SELECTION_PROCESS_NOT_OK;
        };

        ret_val = dynamic_range_measurement(
            h_loudness_info_set,
            &h_uni_drc_config.drc_instructions_uni_drc[p_candidate.inst()],
            p_downmix_id_requested
                [(p_candidate.downmix_id_request_index as usize).min(MAX_REQUESTS_DOWNMIX_ID - 1)],
            dr.measurement_request_type,
            album_mode,
            &mut peak_to_average_present,
            &mut peak_to_average,
        );
        if ret_val != 0 {
            return ret_val;
        }

        if peak_to_average_present != 0 {
            if dr.requested_is_range == 0 {
                let val = f_abs(dr.request_value.wrapping_sub(peak_to_average));

                if min_val > val {
                    min_val = val;

                    selection_set_number(&mut sd[l.sel], num_selected_candidates);
                }
                if !selection_add(&mut sd[l.sel], &p_candidate) {
                    return DRCDEC_SELECTION_PROCESS_NOT_OK;
                }
            } else {
                if (peak_to_average >= dr.request_value_min)
                    && (peak_to_average <= dr.request_value_max)
                {
                    if !selection_add(&mut sd[l.sel], &p_candidate) {
                        return DRCDEC_SELECTION_PROCESS_NOT_OK;
                    }
                }
            }
        }
    }

    ret_val
}

/// `_selectSingleDrcCharacteristic(hUniDrcConfig, requestedDrcCharacteristic,
/// ppCandidatesPotential, ppCandidatesSelected)`
fn select_single_drc_characteristic(
    h_uni_drc_config: &UniDrcConfig,
    requested_drc_characteristic: i32,
    sd: &mut [DrcdecSelection; 2],
    l: &mut Lists,
) -> DrcdecSelectionProcessReturn {
    if requested_drc_characteristic < 1 {
        return DRCDEC_SELECTION_PROCESS_NOT_OK;
    }

    let Some(coef_idx) = select_drc_coefficients(h_uni_drc_config, LOCATION_SELECTED) else {
        /* check for parametricDRC */
        return DRCDEC_SELECTION_PROCESS_NO_ERROR;
    };
    let p_coef = &h_uni_drc_config.drc_coefficients_uni_drc[coef_idx];

    for i in 0..selection_get_number(&sd[l.pot]) {
        let Some(p_candidate) = selection_get_at(&sd[l.pot], i) else {
            return DRCDEC_SELECTION_PROCESS_NOT_OK;
        };

        let p_inst = &h_uni_drc_config.drc_instructions_uni_drc[p_candidate.inst()];

        let mut hit = 0;

        for j in 0..(p_inst.n_drc_channel_groups as usize).min(8) {
            let index_drc_coeff = p_inst.gain_set_index_for_channel_group[j] as i32;

            if index_drc_coeff > p_coef.gain_set_count as i32 - 1 {
                /* check for parametricDRC */
                return DRCDEC_SELECTION_PROCESS_NO_ERROR;
            }

            // indexDrcCoeff >= 0: the channel groups have gain set indices >= 0.
            let p_gain_set = &p_coef.gain_set[(index_drc_coeff.max(0)) as usize];
            let band_count = (p_gain_set.band_count as usize).min(4);

            for b in 0..band_count {
                if (p_gain_set.drc_characteristic[b].is_cicp != 0)
                    && (p_gain_set.drc_characteristic[b].cicp_index as i32
                        == requested_drc_characteristic)
                {
                    hit = 1;
                    break;
                }
            }

            if hit != 0 {
                break;
            }
        }

        if hit != 0 {
            if !selection_add(&mut sd[l.sel], &p_candidate) {
                return DRCDEC_SELECTION_PROCESS_NOT_OK;
            }
        }
    }

    if selection_get_number(&sd[l.sel]) != 0 {
        swap_selection_and_clear(sd, l);
    }

    DRCDEC_SELECTION_PROCESS_NO_ERROR
}

/// `secondTry` of `_selectDrcCharacteristic()` (function-local in C), dumped with (the
/// declaration of the C++ source, made `static`, renamed)
///
/// `scripts/dump_table.py libDRCdec/src/drcDec_selectionProcess.cpp secondTry_fnlocal INT --prelude 'static const int secondTry_fnlocal[12] = {0, 2, 3, 4, 5, 6, 5, 9, 10, 7, 8, 10};'`
pub static SECOND_TRY: [i32; 12] = [0, 2, 3, 4, 5, 6, 5, 9, 10, 7, 8, 10];

/// `_selectDrcCharacteristic(hUniDrcConfig, drcCharacteristicRequested, ppCandidatesPotential,
/// ppCandidatesSelected)`
fn select_drc_characteristic(
    h_uni_drc_config: &UniDrcConfig,
    drc_characteristic_requested: i32,
    sd: &mut [DrcdecSelection; 2],
    l: &mut Lists,
) -> DrcdecSelectionProcessReturn {
    let mut ret_val;

    ret_val =
        select_single_drc_characteristic(h_uni_drc_config, drc_characteristic_requested, sd, l);
    if ret_val != 0 {
        return ret_val;
    }

    if (drc_characteristic_requested <= 11) && (selection_get_number(&sd[l.sel]) == 0) {
        // drcCharacteristicRequested >= 1 here (the first call returns an error otherwise).
        ret_val = select_single_drc_characteristic(
            h_uni_drc_config,
            SECOND_TRY[drc_characteristic_requested.clamp(0, 11) as usize],
            sd,
            l,
        );
        if ret_val != 0 {
            return ret_val;
        }
    }

    if selection_get_number(&sd[l.sel]) == 0 {
        if (drc_characteristic_requested >= 2) && (drc_characteristic_requested <= 5) {
            ret_val = select_single_drc_characteristic(
                h_uni_drc_config,
                drc_characteristic_requested - 1,
                sd,
                l,
            );
            if ret_val != 0 {
                return ret_val;
            }
        } else if drc_characteristic_requested == 11 {
            ret_val = select_single_drc_characteristic(h_uni_drc_config, 9, sd, l);
            if ret_val != 0 {
                return ret_val;
            }
        }
    }

    swap_selection(l);

    ret_val
}

/// `_drcSetFinalSelection_peakValue0(pCandidatesPotential, pCandidatesSelected)`
fn drc_set_final_selection_peak_value0(
    sd: &mut [DrcdecSelection; 2],
    l: Lists,
) -> DrcdecSelectionProcessReturn {
    for i in 0..selection_get_number(&sd[l.pot]) {
        let Some(p_candidate) = selection_get_at(&sd[l.pot], i) else {
            return DRCDEC_SELECTION_PROCESS_NOT_OK;
        };

        if p_candidate.output_peak_level <= 0 {
            if !selection_add(&mut sd[l.sel], &p_candidate) {
                return DRCDEC_SELECTION_PROCESS_NOT_OK;
            }
        }
    }

    DRCDEC_SELECTION_PROCESS_NO_ERROR
}

/// `_drcSetFinalSelection_downmixId(hSelProcInput, ppCandidatesPotential,
/// ppCandidatesSelected)`
fn drc_set_final_selection_downmix_id(
    h_sel_proc_input: &SelProcInput,
    h_uni_drc_config: &UniDrcConfig,
    sd: &mut [DrcdecSelection; 2],
    l: &mut Lists,
) -> DrcdecSelectionProcessReturn {
    for i in 0..selection_get_number(&sd[l.pot]) {
        let Some(p_candidate) = selection_get_at(&sd[l.pot], i) else {
            return DRCDEC_SELECTION_PROCESS_NOT_OK;
        };

        let p_inst = &h_uni_drc_config.drc_instructions_uni_drc[p_candidate.inst()];

        for j in 0..(p_inst.downmix_id_count as usize).min(8) {
            let id = p_inst.downmix_id[j] as i32;
            if DOWNMIX_ID_BASE_LAYOUT != id
                && DOWNMIX_ID_ANY_DOWNMIX != id
                && h_sel_proc_input.downmix_id_requested[(p_candidate.downmix_id_request_index
                    as usize)
                    .min(MAX_REQUESTS_DOWNMIX_ID - 1)] as i32
                    == id
            {
                if !selection_add(&mut sd[l.sel], &p_candidate) {
                    return DRCDEC_SELECTION_PROCESS_NOT_OK;
                }
            }
        }
    }

    if selection_get_number(&sd[l.sel]) == 0 {
        swap_selection(l);
    }

    DRCDEC_SELECTION_PROCESS_NO_ERROR
}

/// `_crossSum(value)`
fn cross_sum(mut value: i32) -> i32 {
    let mut sum = 0;

    while value != 0 {
        if (value & 1) == 1 {
            sum += 1;
        }

        value >>= 1;
    }

    sum
}

/// `_drcSetFinalSelection_effectTypes(pCandidatesPotential, pCandidatesSelected)`
fn drc_set_final_selection_effect_types(
    h_uni_drc_config: &UniDrcConfig,
    sd: &mut [DrcdecSelection; 2],
    l: Lists,
) -> DrcdecSelectionProcessReturn {
    let mut min_num_effects = 1000;

    for i in 0..selection_get_number(&sd[l.pot]) {
        let Some(p_candidate) = selection_get_at(&sd[l.pot], i) else {
            return DRCDEC_SELECTION_PROCESS_NOT_OK;
        };

        let mut effects =
            h_uni_drc_config.drc_instructions_uni_drc[p_candidate.inst()].drc_set_effect as i32;
        effects &= 0xffff ^ (EB_GENERAL_COMPR);
        let num_effects = cross_sum(effects);

        if num_effects < min_num_effects {
            min_num_effects = num_effects;
        }
    }

    /* add all with minimum number of effects */
    for i in 0..selection_get_number(&sd[l.pot]) {
        let Some(p_candidate) = selection_get_at(&sd[l.pot], i) else {
            return DRCDEC_SELECTION_PROCESS_NOT_OK;
        };

        let mut effects =
            h_uni_drc_config.drc_instructions_uni_drc[p_candidate.inst()].drc_set_effect as i32;
        effects &= 0xffff ^ (EB_GENERAL_COMPR);
        let num_effects = cross_sum(effects);

        if num_effects == min_num_effects {
            if !selection_add(&mut sd[l.sel], &p_candidate) {
                return DRCDEC_SELECTION_PROCESS_NOT_OK;
            }
        }
    }

    DRCDEC_SELECTION_PROCESS_NO_ERROR
}

/// `_selectSmallestTargetLoudnessValueUpper(pCandidatesPotential, pCandidatesSelected)`
fn select_smallest_target_loudness_value_upper(
    h_uni_drc_config: &UniDrcConfig,
    sd: &mut [DrcdecSelection; 2],
    l: Lists,
) -> DrcdecSelectionProcessReturn {
    let mut min_val: i8 = 0x7F;

    for i in 0..selection_get_number(&sd[l.pot]) {
        let Some(p_candidate) = selection_get_at(&sd[l.pot], i) else {
            return DRCDEC_SELECTION_PROCESS_NOT_OK;
        };

        let val = h_uni_drc_config.drc_instructions_uni_drc[p_candidate.inst()]
            .drc_set_target_loudness_value_upper;

        if val < min_val {
            min_val = val;
        }
    }

    /* add all with same smallest drcSetTargetLoudnessValueUpper */
    for i in 0..selection_get_number(&sd[l.pot]) {
        let Some(p_candidate) = selection_get_at(&sd[l.pot], i) else {
            return DRCDEC_SELECTION_PROCESS_NOT_OK;
        };

        let val = h_uni_drc_config.drc_instructions_uni_drc[p_candidate.inst()]
            .drc_set_target_loudness_value_upper;

        if val == min_val {
            if !selection_add(&mut sd[l.sel], &p_candidate) {
                return DRCDEC_SELECTION_PROCESS_NOT_OK;
            }
        }
    }

    DRCDEC_SELECTION_PROCESS_NO_ERROR
}

/// `_drcSetFinalSelection_targetLoudness(targetLoudness, pCandidatesPotential,
/// pCandidatesSelected)`
///
/// The candidate lists are swapped locally (C swaps its copies of the pointers), so the caller's
/// lists are passed by value.
fn drc_set_final_selection_target_loudness(
    target_loudness: FixpDbl,
    h_uni_drc_config: &UniDrcConfig,
    sd: &mut [DrcdecSelection; 2],
    mut l: Lists,
) -> DrcdecSelectionProcessReturn {
    let mut ret_val = DRCDEC_SELECTION_PROCESS_NO_ERROR;

    for i in 0..selection_get_number(&sd[l.pot]) {
        let Some(p_candidate) = selection_get_at(&sd[l.pot], i) else {
            return DRCDEC_SELECTION_PROCESS_NOT_OK;
        };

        if p_candidate.selection_flag == 0 {
            if !selection_add(&mut sd[l.sel], &p_candidate) {
                return DRCDEC_SELECTION_PROCESS_NOT_OK;
            }
        }
    }

    if selection_get_number(&sd[l.sel]) == 0 {
        ret_val = select_smallest_target_loudness_value_upper(h_uni_drc_config, sd, l);
        if ret_val != 0 {
            return ret_val;
        }
    }

    if selection_get_number(&sd[l.sel]) > 1 {
        swap_selection_and_clear(sd, &mut l);

        for i in 0..selection_get_number(&sd[l.pot]) {
            let Some(p_candidate) = selection_get_at(&sd[l.pot], i) else {
                return DRCDEC_SELECTION_PROCESS_NOT_OK;
            };

            let p_drc_instruction_uni_drc =
                &h_uni_drc_config.drc_instructions_uni_drc[p_candidate.inst()];

            if target_loudness_in_range(p_drc_instruction_uni_drc, target_loudness) != 0 {
                if !selection_add(&mut sd[l.sel], &p_candidate) {
                    return DRCDEC_SELECTION_PROCESS_NOT_OK;
                }
            }
        }

        if selection_get_number(&sd[l.sel]) > 1 {
            swap_selection_and_clear(sd, &mut l);

            ret_val = select_smallest_target_loudness_value_upper(h_uni_drc_config, sd, l);
            if ret_val != 0 {
                return ret_val;
            }
        }
    }

    ret_val
}

/// `_drcSetFinalSelection_peakValueLargest(pCandidatesPotential, pCandidatesSelected)`
fn drc_set_final_selection_peak_value_largest(
    sd: &mut [DrcdecSelection; 2],
    l: Lists,
) -> DrcdecSelectionProcessReturn {
    let mut largest_peak_level: FixpDbl = MINVAL_DBL;

    for i in 0..selection_get_number(&sd[l.pot]) {
        let Some(p_candidate) = selection_get_at(&sd[l.pot], i) else {
            return DRCDEC_SELECTION_PROCESS_NOT_OK;
        };

        let peak_level = p_candidate.output_peak_level;

        if peak_level > largest_peak_level {
            largest_peak_level = peak_level;
        }
    }

    /* add all with same largest peak level */
    for i in 0..selection_get_number(&sd[l.pot]) {
        let Some(p_candidate) = selection_get_at(&sd[l.pot], i) else {
            return DRCDEC_SELECTION_PROCESS_NOT_OK;
        };

        let peak_level = p_candidate.output_peak_level;

        if peak_level == largest_peak_level {
            if !selection_add(&mut sd[l.sel], &p_candidate) {
                return DRCDEC_SELECTION_PROCESS_NOT_OK;
            }
        }
    }

    DRCDEC_SELECTION_PROCESS_NO_ERROR
}

/// `_drcSetFinalSelection_drcSetId(pCandidatesPotential, pCandidatesSelected)`
fn drc_set_final_selection_drc_set_id(
    h_uni_drc_config: &UniDrcConfig,
    sd: &mut [DrcdecSelection; 2],
    l: Lists,
) -> DrcdecSelectionProcessReturn {
    let mut largest_id = -1000;
    let mut p_candidate_selected: Option<DrcdecSelectionData> = None;

    for i in 0..selection_get_number(&sd[l.pot]) {
        let Some(p_candidate) = selection_get_at(&sd[l.pot], i) else {
            return DRCDEC_SELECTION_PROCESS_NOT_OK;
        };

        let id = h_uni_drc_config.drc_instructions_uni_drc[p_candidate.inst()].drc_set_id as i32;

        if id > largest_id {
            largest_id = id;
            p_candidate_selected = Some(p_candidate);
        }
    }

    if let Some(c) = p_candidate_selected {
        if !selection_add(&mut sd[l.sel], &c) {
            return DRCDEC_SELECTION_PROCESS_NOT_OK;
        }
    } else {
        return DRCDEC_SELECTION_PROCESS_NOT_OK;
    }

    DRCDEC_SELECTION_PROCESS_NO_ERROR
}

/// `_drcSetFinalSelection(hSelProcInput, hUniDrcConfig, ppCandidatesPotential,
/// ppCandidatesSelected, codecMode)`
fn drc_set_final_selection(
    h_sel_proc_input: &SelProcInput,
    h_uni_drc_config: &UniDrcConfig,
    sd: &mut [DrcdecSelection; 2],
    l: &mut Lists,
    _codec_mode: SelProcCodecMode,
) -> DrcdecSelectionProcessReturn {
    let mut ret_val = DRCDEC_SELECTION_PROCESS_NO_ERROR;

    if selection_get_number(&sd[l.pot]) == 0 {
        return DRCDEC_SELECTION_PROCESS_NOT_OK;
    } else if selection_get_number(&sd[l.pot]) == 1 {
        swap_selection(l);
        /* finished */
    } else {
        /* > 1 */
        ret_val = drc_set_final_selection_peak_value0(sd, *l);
        if ret_val != 0 {
            return ret_val;
        }

        if selection_get_number(&sd[l.sel]) > 1 {
            swap_selection_and_clear(sd, l);
            ret_val = drc_set_final_selection_downmix_id(h_sel_proc_input, h_uni_drc_config, sd, l);
            if ret_val != 0 {
                return ret_val;
            }
        }

        if selection_get_number(&sd[l.sel]) > 1 {
            swap_selection_and_clear(sd, l);
            ret_val = drc_set_final_selection_effect_types(h_uni_drc_config, sd, *l);
            if ret_val != 0 {
                return ret_val;
            }
        }

        if selection_get_number(&sd[l.sel]) > 1 {
            swap_selection_and_clear(sd, l);
            ret_val = drc_set_final_selection_target_loudness(
                h_sel_proc_input.target_loudness,
                h_uni_drc_config,
                sd,
                *l,
            );
            if ret_val != 0 {
                return ret_val;
            }
        }

        if selection_get_number(&sd[l.sel]) > 1 {
            swap_selection_and_clear(sd, l);
            ret_val = drc_set_final_selection_peak_value_largest(sd, *l);
            if ret_val != 0 {
                return ret_val;
            }
        }

        if selection_get_number(&sd[l.sel]) > 1 {
            swap_selection_and_clear(sd, l);
            ret_val = drc_set_final_selection_drc_set_id(h_uni_drc_config, sd, *l);
            if ret_val != 0 {
                return ret_val;
            }
        }
    }

    if selection_get_number(&sd[l.sel]) == 0 {
        return DRCDEC_SELECTION_PROCESS_NOT_OK;
    }

    ret_val
}

/// `_generateVirtualDrcSets(hSelProcInput, hUniDrcConfig, codecMode)`
fn generate_virtual_drc_sets(
    h_sel_proc_input: &SelProcInput,
    h_uni_drc_config: &mut UniDrcConfig,
    codec_mode: SelProcCodecMode,
) -> DrcdecSelectionProcessReturn {
    let mut n_mixes = h_uni_drc_config.downmix_instructions_count as usize + 1;
    let mut index = h_uni_drc_config.drc_instructions_uni_drc_count as usize;
    let mut index_virtual: i32 = -1;

    if codec_mode == SEL_PROC_MPEG_H_3DA {
        n_mixes = 1;
    }

    if (index + n_mixes) > (12 + 1 + 6) {
        return DRCDEC_SELECTION_PROCESS_NOT_OK;
    }

    let p_drc_instruction = &mut h_uni_drc_config.drc_instructions_uni_drc[index];
    *p_drc_instruction = DrcInstructionsUniDrc::ZERO;

    p_drc_instruction.drc_set_id = index_virtual as i8;
    index += 1;
    index_virtual -= 1;
    p_drc_instruction.downmix_id_count = 1;

    if (codec_mode == SEL_PROC_MPEG_H_3DA) && (h_sel_proc_input.num_downmix_id_requests != 0) {
        p_drc_instruction.downmix_id[0] = h_sel_proc_input.downmix_id_requested[0];
    } else {
        p_drc_instruction.downmix_id[0] = DOWNMIX_ID_BASE_LAYOUT as u8;
    }

    for i in 1..n_mixes {
        let downmix_id = h_uni_drc_config.downmix_instructions[i - 1].downmix_id;
        let p_drc_instruction = &mut h_uni_drc_config.drc_instructions_uni_drc[index];
        *p_drc_instruction = DrcInstructionsUniDrc::ZERO;
        p_drc_instruction.drc_set_id = index_virtual as i8;
        p_drc_instruction.downmix_id[0] = downmix_id;
        p_drc_instruction.downmix_id_count = 1;
        index += 1;
        index_virtual -= 1;
    }

    h_uni_drc_config.drc_instructions_count_incl_virtual =
        (h_uni_drc_config.drc_instructions_uni_drc_count as usize + n_mixes) as u8;

    DRCDEC_SELECTION_PROCESS_NO_ERROR
}

/// `_generateOutputInfo(hSelProcInput, hSelProcOutput, hUniDrcConfig, hLoudnessInfoSet,
/// pSelectionData, codecMode)`
///
/// Index 3 of `selectedDrcSetIds` and `selectedDownmixIds` reaches the next field of the C
/// structure; reproduced from the C layout.
fn generate_output_info(
    h_sel_proc_input: &SelProcInput,
    h_sel_proc_output: &mut SelProcOutput,
    h_uni_drc_config: &UniDrcConfig,
    h_loudness_info_set: &LoudnessInfoSet,
    p_selection_data: &DrcdecSelectionData,
    _codec_mode: SelProcCodecMode,
) -> DrcdecSelectionProcessReturn {
    let ret_val = DRCDEC_SELECTION_PROCESS_NO_ERROR;

    let mut has_dependend = 0u8;
    let mut has_fading = 0u8;
    let mut has_ducking = 0;
    let mut mixing_level: FixpDbl = 0;
    let album_mode = h_sel_proc_input.album_mode as i32;
    let p_downmix_id_requested = &h_sel_proc_input.downmix_id_requested;
    let boost = h_sel_proc_input.boost;
    let compress = h_sel_proc_input.compress;
    let p_inst_sel = &h_uni_drc_config.drc_instructions_uni_drc[p_selection_data.inst()];

    h_sel_proc_output.num_selected_drc_sets = 1;
    h_sel_proc_output.selected_drc_set_ids[0] = p_inst_sel.drc_set_id;
    h_sel_proc_output.selected_downmix_ids[0] =
        if p_inst_sel.drc_apply_to_downmix == 1 { p_inst_sel.downmix_id[0] } else { 0 };
    h_sel_proc_output.loudness_normalization_gain_db = p_selection_data
        .loudness_normalization_gain_db_adjusted
        .wrapping_add(h_sel_proc_input.loudness_normalization_gain_modification_db);
    h_sel_proc_output.output_peak_level_db = p_selection_data.output_peak_level;
    h_sel_proc_output.output_loudness = p_selection_data.output_loudness;

    h_sel_proc_output.boost = boost;
    h_sel_proc_output.compress = compress;
    h_sel_proc_output.base_channel_count = h_uni_drc_config.channel_layout.base_channel_count;
    h_sel_proc_output.target_channel_count = h_uni_drc_config.channel_layout.base_channel_count;
    h_sel_proc_output.active_downmix_id = p_downmix_id_requested
        [(p_selection_data.downmix_id_request_index as usize).min(MAX_REQUESTS_DOWNMIX_ID - 1)];

    get_mixing_level(
        h_loudness_info_set,
        p_downmix_id_requested[0] as i32,
        h_sel_proc_output.selected_drc_set_ids[0] as i32,
        album_mode,
        &mut mixing_level,
    );
    h_sel_proc_output.mixing_level = mixing_level;

    /*dependent*/
    if p_inst_sel.depends_on_drc_set_present != 0 {
        let depends_on_drc_set_id = p_inst_sel.depends_on_drc_set as i32;

        for i in 0..h_uni_drc_config.drc_instructions_count_incl_virtual as usize {
            let p_inst = &h_uni_drc_config.drc_instructions_uni_drc[i];
            if drc_set_is_usable(h_uni_drc_config, i) == 0 {
                continue;
            }

            if p_inst.drc_set_id as i32 == depends_on_drc_set_id {
                let n = h_sel_proc_output.num_selected_drc_sets as usize;
                h_sel_proc_output.set_selected_drc_set_id(n, p_inst.drc_set_id);
                h_sel_proc_output.set_selected_downmix_id(
                    n,
                    if p_inst.drc_apply_to_downmix == 1 { p_inst.downmix_id[0] } else { 0 },
                );
                h_sel_proc_output.num_selected_drc_sets += 1;
                has_dependend = 1;
                break;
            }
        }
    }

    /* fading */
    if h_sel_proc_input.album_mode == 0 {
        for i in 0..h_uni_drc_config.drc_instructions_uni_drc_count as usize {
            let p_inst = &h_uni_drc_config.drc_instructions_uni_drc[i];
            if drc_set_is_usable(h_uni_drc_config, i) == 0 {
                continue;
            }

            if p_inst.drc_set_effect as i32 & EB_FADE != 0 {
                if p_inst.downmix_id[0] as i32 == DOWNMIX_ID_ANY_DOWNMIX {
                    h_sel_proc_output.num_selected_drc_sets = has_dependend + 1;
                    let n = h_sel_proc_output.num_selected_drc_sets as usize;
                    h_sel_proc_output.set_selected_drc_set_id(n, p_inst.drc_set_id);
                    h_sel_proc_output.set_selected_downmix_id(
                        n,
                        if p_inst.drc_apply_to_downmix == 1 { p_inst.downmix_id[0] } else { 0 },
                    );
                    h_sel_proc_output.num_selected_drc_sets += 1;
                    has_fading = 1;
                } else {
                    return DRCDEC_SELECTION_PROCESS_NOT_OK;
                }
            }
        }
    }

    /* ducking */
    for i in 0..h_uni_drc_config.drc_instructions_uni_drc_count as usize {
        let p_inst = &h_uni_drc_config.drc_instructions_uni_drc[i];
        if drc_set_is_usable(h_uni_drc_config, i) == 0 {
            continue;
        }

        if p_inst.drc_set_effect as i32 & (EB_DUCK_OTHER | EB_DUCK_SELF) != 0 {
            for j in 0..(p_inst.downmix_id_count as usize).min(8) {
                if p_inst.downmix_id[j] == h_sel_proc_output.active_downmix_id {
                    /* ducking overrides fading */
                    h_sel_proc_output.num_selected_drc_sets = has_dependend + 1;

                    let n = h_sel_proc_output.num_selected_drc_sets as usize;
                    h_sel_proc_output.set_selected_drc_set_id(n, p_inst.drc_set_id);
                    /* force ducking DRC set to be processed on base layout */
                    h_sel_proc_output.set_selected_downmix_id(n, 0);
                    h_sel_proc_output.num_selected_drc_sets += 1;
                    has_ducking = 1;
                }
            }
        }
    }

    /* repeat for DOWNMIX_ID_BASE_LAYOUT if no ducking found*/

    if has_ducking == 0 {
        for i in 0..h_uni_drc_config.drc_instructions_uni_drc_count as usize {
            let p_inst = &h_uni_drc_config.drc_instructions_uni_drc[i];
            if drc_set_is_usable(h_uni_drc_config, i) == 0 {
                continue;
            }

            if p_inst.drc_set_effect as i32 & (EB_DUCK_OTHER | EB_DUCK_SELF) != 0 {
                for j in 0..(p_inst.downmix_id_count as usize).min(8) {
                    if p_inst.downmix_id[j] as i32 == DOWNMIX_ID_BASE_LAYOUT {
                        h_sel_proc_output.num_selected_drc_sets = has_dependend + has_fading + 1;
                        let n = h_sel_proc_output.num_selected_drc_sets as usize;
                        h_sel_proc_output.set_selected_drc_set_id(n, p_inst.drc_set_id);
                        /* force ducking DRC set to be processed on base layout */
                        h_sel_proc_output.set_selected_downmix_id(n, 0);
                        h_sel_proc_output.num_selected_drc_sets += 1;
                    }
                }
            }
        }
    }

    if h_sel_proc_output.num_selected_drc_sets > 3 {
        /* maximum permitted number of applied DRC sets is 3, see section 6.3.5 of ISO/IEC
         * 23003-4 */
        h_sel_proc_output.num_selected_drc_sets = 0;
        return DRCDEC_SELECTION_PROCESS_NOT_OK;
    }

    /* sorting: Ducking/Fading -> Dependent -> Selected */
    if h_sel_proc_output.num_selected_drc_sets == 3 {
        h_sel_proc_output.selected_drc_set_ids.swap(0, 2);
        h_sel_proc_output.selected_downmix_ids.swap(0, 2);
    } else if h_sel_proc_output.num_selected_drc_sets == 2 {
        h_sel_proc_output.selected_drc_set_ids.swap(0, 1);
        h_sel_proc_output.selected_downmix_ids.swap(0, 1);
    }

    ret_val
}

/// `_selectDownmixMatrix(hSelProcOutput, hUniDrcConfig)`
fn select_downmix_matrix(
    h_sel_proc_output: &mut SelProcOutput,
    h_uni_drc_config: &UniDrcConfig,
) -> DrcdecSelectionProcessReturn {
    h_sel_proc_output.base_channel_count = h_uni_drc_config.channel_layout.base_channel_count;
    h_sel_proc_output.target_channel_count = h_uni_drc_config.channel_layout.base_channel_count;
    h_sel_proc_output.target_layout = -1;
    h_sel_proc_output.downmix_matrix_present = 0;

    if h_sel_proc_output.active_downmix_id != 0 {
        for i in 0..h_uni_drc_config.downmix_instructions_count.min(6) as usize {
            let p_down = &h_uni_drc_config.downmix_instructions[i];
            if p_down.target_channel_count > 8 {
                continue;
            }

            if h_sel_proc_output.active_downmix_id == p_down.downmix_id {
                h_sel_proc_output.target_channel_count = p_down.target_channel_count;
                h_sel_proc_output.target_layout = p_down.target_layout as i8;

                if p_down.downmix_coefficients_present != 0 {
                    let downmix_offset =
                        get_downmix_offset(p_down, h_sel_proc_output.base_channel_count as i32); /* e = 1 */

                    // baseChannelCount <= 8 (reader), targetChannelCount <= 8 (checked above).
                    let base = (h_sel_proc_output.base_channel_count as usize).min(8);
                    let target = h_sel_proc_output.target_channel_count as usize;
                    for j in 0..base {
                        for k in 0..target {
                            h_sel_proc_output.downmix_matrix[j][k] = f_mult_div2(
                                downmix_offset,
                                p_down.downmix_coefficient[j + k * base],
                            )
                            .wrapping_shl(2);
                        }
                    }

                    h_sel_proc_output.downmix_matrix_present = 1;
                }
                break;
            }
        }
    }

    DRCDEC_SELECTION_PROCESS_NO_ERROR
}

/// `_drcSetPreSelection(hSelProcInput, hUniDrcConfig, hLoudnessInfoSet, ppCandidatesPotential,
/// ppCandidatesSelected, codecMode)`
fn drc_set_pre_selection(
    h_sel_proc_input: &SelProcInput,
    h_uni_drc_config: &UniDrcConfig,
    h_loudness_info_set: &LoudnessInfoSet,
    sd: &mut [DrcdecSelection; 2],
    l: &mut Lists,
    codec_mode: SelProcCodecMode,
) -> DrcdecSelectionProcessReturn {
    let mut ret_val;

    // numDownmixIdRequests is at most 7 (1 + downmixInstructionsCount).
    for i in 0..(h_sel_proc_input.num_downmix_id_requests as usize).min(MAX_REQUESTS_DOWNMIX_ID) {
        for j in 0..h_uni_drc_config.drc_instructions_count_incl_virtual as usize {
            /* check if ID is unique */
            if select_drc_instructions(
                h_uni_drc_config,
                h_uni_drc_config.drc_instructions_uni_drc[j].drc_set_id as i32,
            ) != Some(j)
            {
                continue;
            }

            ret_val = drc_set_pre_selection_single_instruction(
                h_sel_proc_input,
                i,
                h_uni_drc_config,
                h_loudness_info_set,
                j,
                sd,
                *l,
                codec_mode,
            );
            if ret_val != 0 {
                return DRCDEC_SELECTION_PROCESS_NOT_OK;
            }
        }
    }

    ret_val = pre_selection_requirement9(h_sel_proc_input, sd, *l);
    if ret_val != 0 {
        return DRCDEC_SELECTION_PROCESS_NOT_OK;
    }

    if selection_get_number(&sd[l.sel]) == 0 {
        ret_val = drc_set_selection_add_candidates(h_sel_proc_input, h_uni_drc_config, sd, *l);
        if ret_val != 0 {
            return DRCDEC_SELECTION_PROCESS_NOT_OK;
        }
    }

    ret_val
}

/// `_drcSetRequestSelection(hSelProcInput, hUniDrcConfig, hLoudnessInfoSet,
/// ppCandidatesPotential, ppCandidatesSelected)`
fn drc_set_request_selection(
    h_sel_proc_input: &SelProcInput,
    h_uni_drc_config: &UniDrcConfig,
    h_loudness_info_set: &LoudnessInfoSet,
    sd: &mut [DrcdecSelection; 2],
    l: &mut Lists,
) -> DrcdecSelectionProcessReturn {
    let mut ret_val;

    if selection_get_number(&sd[l.pot]) == 0 {
        return DRCDEC_SELECTION_PROCESS_NOT_OK;
    }

    if h_sel_proc_input.dynamic_range_control_on != 0 {
        if h_sel_proc_input.num_drc_feature_requests == 0 {
            ret_val = select_drc_set_effect_none(h_uni_drc_config, sd, *l);
            if ret_val != 0 {
                return ret_val;
            }

            if selection_get_number(&sd[l.sel]) == 0 {
                // Only the drcEffectType member of the local request is initialized in C; it is
                // passed by value and only that member is read.
                let mut fallback_request = DrcFeatureRequest::ZERO;
                let et = fallback_request.drc_effect_type_mut();
                et.num_requests = 5;
                et.num_requests_desired = 5;
                et.request[0] = DETR_GENERAL_COMPR;
                et.request[1] = DETR_NIGHT;
                et.request[2] = DETR_NOISY;
                et.request[3] = DETR_LIMITED;
                et.request[4] = DETR_LOWLEVEL;

                ret_val = select_effect_type_feature(h_uni_drc_config, fallback_request, sd, l);
                if ret_val != 0 {
                    return DRCDEC_SELECTION_PROCESS_NOT_OK;
                }
            }

            swap_selection_and_clear(sd, l);
        } else {
            for i in 0..(h_sel_proc_input.num_drc_feature_requests as usize)
                .min(MAX_REQUESTS_DRC_FEATURE)
            {
                if h_sel_proc_input.drc_feature_request_type[i] == DFRT_EFFECT_TYPE {
                    ret_val = select_effect_type_feature(
                        h_uni_drc_config,
                        h_sel_proc_input.drc_feature_request[i],
                        sd,
                        l,
                    );

                    swap_selection_and_clear(sd, l);
                    if ret_val != 0 {
                        return DRCDEC_SELECTION_PROCESS_NOT_OK;
                    }
                } else if h_sel_proc_input.drc_feature_request_type[i] == DFRT_DYNAMIC_RANGE {
                    ret_val = select_dynamic_range(
                        h_uni_drc_config,
                        h_loudness_info_set,
                        h_sel_proc_input.drc_feature_request[i],
                        &h_sel_proc_input.downmix_id_requested,
                        h_sel_proc_input.album_mode as i32,
                        sd,
                        *l,
                    );

                    if selection_get_number(&sd[l.sel]) > 0 {
                        swap_selection_and_clear(sd, l);
                    }
                    if ret_val != 0 {
                        return DRCDEC_SELECTION_PROCESS_NOT_OK;
                    }
                } else if h_sel_proc_input.drc_feature_request_type[i] == DFRT_DRC_CHARACTERISTIC {
                    ret_val = select_drc_characteristic(
                        h_uni_drc_config,
                        h_sel_proc_input.drc_feature_request[i].drc_characteristic() as i32,
                        sd,
                        l,
                    );

                    if selection_get_number(&sd[l.sel]) > 0 {
                        swap_selection_and_clear(sd, l);
                    }
                    if ret_val != 0 {
                        return DRCDEC_SELECTION_PROCESS_NOT_OK;
                    }
                }
            }
        }
    }

    DRCDEC_SELECTION_PROCESS_NO_ERROR
}

/*******************************************/

/// `_dynamicRangeMeasurement(hLoudnessInfoSet, pInst, downmixIdRequested,
/// dynamicRangeMeasurementType, albumMode, pPeakToAveragePresent, pPeakToAverage)`
fn dynamic_range_measurement(
    h_loudness_info_set: &LoudnessInfoSet,
    p_inst: &DrcInstructionsUniDrc,
    downmix_id_requested: u8,
    dynamic_range_measurement_type: DynRangeMeasurementRequestType,
    album_mode: i32,
    p_peak_to_average_present: &mut i32,
    p_peak_to_average: &mut FixpDbl,
) -> DrcdecSelectionProcessReturn {
    let mut ret_val = DRCDEC_SELECTION_PROCESS_NO_ERROR;
    let drc_set_id = (p_inst.drc_set_id as i32).max(0);

    *p_peak_to_average_present = 0;

    if album_mode != 0 {
        for i in 0..h_loudness_info_set.loudness_info_album_count.min(12) as usize {
            let p_loudness_info = &h_loudness_info_set.loudness_info_album[i];

            if drc_set_id == p_loudness_info.drc_set_id as i32 {
                if downmix_id_requested == p_loudness_info.downmix_id {
                    ret_val = extract_loudness_peak_to_average_value(
                        p_loudness_info,
                        dynamic_range_measurement_type,
                        p_peak_to_average_present,
                        p_peak_to_average,
                    );
                    if ret_val != 0 {
                        return ret_val;
                    }
                }
            }
        }
    }

    if *p_peak_to_average_present == 0 {
        for i in 0..h_loudness_info_set.loudness_info_count.min(12) as usize {
            let p_loudness_info = &h_loudness_info_set.loudness_info[i];

            if drc_set_id == p_loudness_info.drc_set_id as i32 {
                if downmix_id_requested == p_loudness_info.downmix_id {
                    ret_val = extract_loudness_peak_to_average_value(
                        p_loudness_info,
                        dynamic_range_measurement_type,
                        p_peak_to_average_present,
                        p_peak_to_average,
                    );
                    if ret_val != 0 {
                        return ret_val;
                    }
                }
            }
        }
    }

    ret_val
}

/*******************************************/

/// The loudness infos of the album or the track (`count`, `pLoudnessInfo`).
fn loudness_infos(h_loudness_info_set: &LoudnessInfoSet, album_mode: i32) -> &[LoudnessInfo] {
    // The counts are saturated to 12 by the reader.
    if album_mode != 0 {
        let count = h_loudness_info_set.loudness_info_album_count.min(12) as usize;
        &h_loudness_info_set.loudness_info_album[..count]
    } else {
        let count = h_loudness_info_set.loudness_info_count.min(12) as usize;
        &h_loudness_info_set.loudness_info[..count]
    }
}

/// `_getLoudnessInfoStructure(hLoudnessInfoSet, drcSetId, downmixId, albumMode)`
fn get_loudness_info_structure(
    h_loudness_info_set: &LoudnessInfoSet,
    drc_set_id: i32,
    downmix_id: i32,
    album_mode: i32,
) -> Option<&LoudnessInfo> {
    for li in loudness_infos(h_loudness_info_set, album_mode) {
        if (li.drc_set_id as i32 == drc_set_id) && (li.downmix_id as i32 == downmix_id) {
            for j in 0..(li.measurement_count as usize).min(8) {
                let md = li.loudness_measurement[j].method_definition;
                if (md == 1) || (md == 2) {
                    return Some(li);
                }
            }
        }
    }

    None
}

/// `_getApplicableLoudnessInfoStructure(hLoudnessInfoSet, drcSetId, downmixIdRequested,
/// albumMode)`
fn get_applicable_loudness_info_structure(
    h_loudness_info_set: &LoudnessInfoSet,
    drc_set_id: i32,
    downmix_id_requested: i32,
    album_mode: i32,
) -> Option<&LoudnessInfo> {
    let tries: [(i32, i32); 9] = [
        /* default value */
        (drc_set_id, downmix_id_requested),
        /* fallback values */
        (drc_set_id, 0x7F),
        (0x3F, downmix_id_requested),
        (0, downmix_id_requested),
        (0x3F, 0x7F),
        (0, 0x7F),
        (drc_set_id, 0),
        (0x3F, 0),
        (0, 0),
    ];
    for (d, m) in tries {
        let p = get_loudness_info_structure(h_loudness_info_set, d, m, album_mode);
        if p.is_some() {
            return p;
        }
    }
    None
}

/*******************************************/

/// `VALUE_ORDER`
#[derive(Clone, Copy)]
struct ValueOrder {
    value: FixpDbl,
    order: i32,
}

/// `_initValueOrder(pValue)`
fn init_value_order() -> ValueOrder {
    ValueOrder { value: 0, order: -1 }
}

const MS_PROGRAMLOUDNESS: i32 = 9;
const MS_PEAKLOUDNESS: i32 = 10;

/// `pOrdering` of `_getMethodValue()` (function-local in C), dumped with (the declarations of
/// the C++ source, made `static const`, the table renamed)
///
/// `scripts/dump_table.py libDRCdec/src/drcDec_selectionProcess.cpp pOrdering_fnlocal INT --per-line 12 --prelude 'static const int rows = 11; static const int columns = 12; static const int pOrdering_fnlocal[rows][columns] = { {0, 0, 8, 0, 1, 3, 0, 5, 6, 7, 4, 2}, {0, 0, 8, 0, 1, 3, 0, 5, 6, 7, 4, 2}, {0, 0, 1, 0, 8, 5, 0, 2, 3, 4, 6, 7}, {0, 0, 3, 0, 1, 8, 0, 4, 5, 6, 7, 2}, {0, 0, 5, 0, 1, 3, 0, 8, 6, 7, 4, 2}, {0, 0, 5, 0, 1, 3, 0, 6, 8, 7, 4, 2}, {0, 0, 5, 0, 1, 3, 0, 6, 7, 8, 4, 2}, {0, 0, 3, 0, 1, 7, 0, 4, 5, 6, 8, 2}, {0, 0, 1, 0, 7, 5, 0, 2, 3, 4, 6, 8}, {0, 0, 1, 0, 0, 0, 0, 2, 3, 4, 0, 0}, {0, 7, 0, 0, 0, 0, 6, 5, 4, 3, 2, 1} };'`
pub static P_ORDERING: [[i32; 12]; 11] = [
    [0, 0, 8, 0, 1, 3, 0, 5, 6, 7, 4, 2], /* default = bonus1770 */
    [0, 0, 8, 0, 1, 3, 0, 5, 6, 7, 4, 2], /* bonus1770 */
    [0, 0, 1, 0, 8, 5, 0, 2, 3, 4, 6, 7], /* bonusUser */
    [0, 0, 3, 0, 1, 8, 0, 4, 5, 6, 7, 2], /* bonusExpert */
    [0, 0, 5, 0, 1, 3, 0, 8, 6, 7, 4, 2], /* ResA */
    [0, 0, 5, 0, 1, 3, 0, 6, 8, 7, 4, 2], /* ResB */
    [0, 0, 5, 0, 1, 3, 0, 6, 7, 8, 4, 2], /* ResC */
    [0, 0, 3, 0, 1, 7, 0, 4, 5, 6, 8, 2], /* ResD */
    [0, 0, 1, 0, 7, 5, 0, 2, 3, 4, 6, 8], /* ResE */
    [0, 0, 1, 0, 0, 0, 0, 2, 3, 4, 0, 0], /* ProgramLoudness */
    [0, 7, 0, 0, 0, 0, 6, 5, 4, 3, 2, 1], /* PeakLoudness */
];

/// `_getMethodValue(pValueOrder, value, measurementSystem, measurementSystemRequested)`
fn get_method_value(
    p_value_order: &mut ValueOrder,
    value: FixpDbl,
    measurement_system: i32,
    measurement_system_requested: i32,
) -> DrcdecSelectionProcessReturn {
    const ROWS: i32 = 11;
    const COLUMNS: i32 = 12;

    if measurement_system_requested < 0
        || measurement_system_requested >= ROWS
        || measurement_system < 0
        || measurement_system >= COLUMNS
    {
        return DRCDEC_SELECTION_PROCESS_NOT_OK;
    }

    let o = P_ORDERING[measurement_system_requested as usize][measurement_system as usize];
    if o > p_value_order.order {
        p_value_order.order = o;
        p_value_order.value = value;
    }

    DRCDEC_SELECTION_PROCESS_NO_ERROR
}

/*******************************************/

/// `_getLoudness(hLoudnessInfoSet, albumMode, measurementMethodRequested,
/// measurementSystemRequested, targetLoudness, drcSetId, downmixIdRequested,
/// pLoudnessNormalizationGain, pLoudness)`
fn get_loudness(
    h_loudness_info_set: &LoudnessInfoSet,
    album_mode: i32,
    measurement_method_requested: MethodDefinitionRequest,
    measurement_system_requested: MeasurementSystemRequest,
    target_loudness: FixpDbl, /* e = 7 */
    mut drc_set_id: i32,
    downmix_id_requested: i32,
    p_loudness_normalization_gain: &mut FixpDbl, /* e = 7 */
    p_loudness: &mut FixpDbl,                    /* e = 7 */
) -> DrcdecSelectionProcessReturn {
    /* map MDR_DEFAULT to MDR_PROGRAM_LOUDNESS */
    let requested_method_definition = if measurement_method_requested < MDR_ANCHOR_LOUDNESS {
        MDR_PROGRAM_LOUDNESS
    } else {
        MDR_ANCHOR_LOUDNESS
    };

    if measurement_method_requested > MDR_ANCHOR_LOUDNESS {
        return DRCDEC_SELECTION_PROCESS_NOT_OK;
    }

    let mut value_order = init_value_order();

    *p_loudness = UNDEFINED_LOUDNESS_VALUE;
    *p_loudness_normalization_gain = 0;

    if drc_set_id < 0 {
        drc_set_id = 0;
    }

    let mut p_loudness_info = get_applicable_loudness_info_structure(
        h_loudness_info_set,
        drc_set_id,
        downmix_id_requested,
        album_mode,
    );

    if album_mode != 0 && p_loudness_info.is_none() {
        p_loudness_info = get_applicable_loudness_info_structure(
            h_loudness_info_set,
            drc_set_id,
            downmix_id_requested,
            0,
        );
    }

    let Some(p_loudness_info) = p_loudness_info else {
        return DRCDEC_SELECTION_PROCESS_NO_ERROR;
    };

    let mut index: i32 = -1;

    loop {
        index = find_method_definition(p_loudness_info, requested_method_definition, index + 1);

        if index >= 0 {
            let m = &p_loudness_info.loudness_measurement[index as usize];
            get_method_value(
                &mut value_order,
                m.method_value,
                m.measurement_system as i32,
                measurement_system_requested,
            );
        }
        if index < 0 {
            break;
        }
    }

    /* repeat with other method definition */
    if value_order.order == -1 {
        index = -1;

        loop {
            index = find_method_definition(
                p_loudness_info,
                if requested_method_definition == MDR_PROGRAM_LOUDNESS {
                    MDR_ANCHOR_LOUDNESS
                } else {
                    MDR_PROGRAM_LOUDNESS
                },
                index + 1,
            );

            if index >= 0 {
                let m = &p_loudness_info.loudness_measurement[index as usize];
                get_method_value(
                    &mut value_order,
                    m.method_value,
                    m.measurement_system as i32,
                    measurement_system_requested,
                );
            }
            if index < 0 {
                break;
            }
        }
    }

    if value_order.order == -1 {
        return DRCDEC_SELECTION_PROCESS_NOT_OK;
    } else {
        *p_loudness_normalization_gain = target_loudness.wrapping_sub(value_order.value);
        *p_loudness = value_order.value;
    }

    DRCDEC_SELECTION_PROCESS_NO_ERROR
}

/*******************************************/

/// `_truePeakLevelIsPresent(hLoudnessInfoSet, drcSetId, downmixId, albumMode)`
fn true_peak_level_is_present(
    h_loudness_info_set: &LoudnessInfoSet,
    drc_set_id: i32,
    downmix_id: i32,
    album_mode: i32,
) -> i32 {
    for li in loudness_infos(h_loudness_info_set, album_mode) {
        if (li.drc_set_id as i32 == drc_set_id) && (li.downmix_id as i32 == downmix_id) {
            if li.true_peak_level_present != 0 {
                return 1;
            }
        }
    }

    0
}

/// `_getTruePeakLevel(hLoudnessInfoSet, drcSetId, downmixId, albumMode, pTruePeakLevel)`
fn get_true_peak_level(
    h_loudness_info_set: &LoudnessInfoSet,
    drc_set_id: i32,
    downmix_id: i32,
    album_mode: i32,
    p_true_peak_level: &mut FixpDbl,
) -> DrcdecSelectionProcessReturn {
    for li in loudness_infos(h_loudness_info_set, album_mode) {
        if (li.drc_set_id as i32 == drc_set_id) && (li.downmix_id as i32 == downmix_id) {
            if li.true_peak_level_present != 0 {
                *p_true_peak_level = li.true_peak_level;
                return DRCDEC_SELECTION_PROCESS_NO_ERROR;
            }
        }
    }

    DRCDEC_SELECTION_PROCESS_NOT_OK
}

/// `_samplePeakLevelIsPresent(hLoudnessInfoSet, drcSetId, downmixId, albumMode)`
fn sample_peak_level_is_present(
    h_loudness_info_set: &LoudnessInfoSet,
    drc_set_id: i32,
    downmix_id: i32,
    album_mode: i32,
) -> i32 {
    for li in loudness_infos(h_loudness_info_set, album_mode) {
        if (li.drc_set_id as i32 == drc_set_id) && (li.downmix_id as i32 == downmix_id) {
            if li.sample_peak_level_present != 0 {
                return 1;
            }
        }
    }

    0
}

/// `_getSamplePeakLevel(hLoudnessInfoSet, drcSetId, downmixId, albumMode, pSamplePeakLevel)`
fn get_sample_peak_level(
    h_loudness_info_set: &LoudnessInfoSet,
    drc_set_id: i32,
    downmix_id: i32,
    album_mode: i32,
    p_sample_peak_level: &mut FixpDbl, /* e = 7 */
) -> DrcdecSelectionProcessReturn {
    for li in loudness_infos(h_loudness_info_set, album_mode) {
        if (li.drc_set_id as i32 == drc_set_id) && (li.downmix_id as i32 == downmix_id) {
            if li.sample_peak_level_present != 0 {
                *p_sample_peak_level = li.sample_peak_level;
                return DRCDEC_SELECTION_PROCESS_NO_ERROR;
            }
        }
    }

    DRCDEC_SELECTION_PROCESS_NOT_OK
}

/// `_limiterPeakTargetIsPresent(pDrcInstruction, drcSetId, downmixId)`
fn limiter_peak_target_is_present(
    p_drc_instruction: &DrcInstructionsUniDrc,
    _drc_set_id: i32,
    downmix_id: i32,
) -> i32 {
    if p_drc_instruction.limiter_peak_target_present != 0 {
        if (p_drc_instruction.downmix_id[0] as i32 == downmix_id)
            || (p_drc_instruction.downmix_id[0] == 0x7F)
        {
            return 1;
        }

        for i in 0..(p_drc_instruction.downmix_id_count as usize).min(8) {
            if p_drc_instruction.downmix_id[i] as i32 == downmix_id {
                return 1;
            }
        }
    }

    0
}

/// `_getLimiterPeakTarget(pDrcInstruction, drcSetId, downmixId, pLimiterPeakTarget)`
fn get_limiter_peak_target(
    p_drc_instruction: &DrcInstructionsUniDrc,
    _drc_set_id: i32,
    downmix_id: i32,
    p_limiter_peak_target: &mut FixpDbl,
) -> DrcdecSelectionProcessReturn {
    if p_drc_instruction.limiter_peak_target_present != 0 {
        if (p_drc_instruction.downmix_id[0] as i32 == downmix_id)
            || (p_drc_instruction.downmix_id[0] == 0x7F)
        {
            *p_limiter_peak_target = ((p_drc_instruction.limiter_peak_target as i32) << 16) >> 2;
            return DRCDEC_SELECTION_PROCESS_NO_ERROR;
        }

        for i in 0..(p_drc_instruction.downmix_id_count as usize).min(8) {
            if p_drc_instruction.downmix_id[i] as i32 == downmix_id {
                *p_limiter_peak_target =
                    ((p_drc_instruction.limiter_peak_target as i32) << 16) >> 2;
                return DRCDEC_SELECTION_PROCESS_NO_ERROR;
            }
        }
    }

    DRCDEC_SELECTION_PROCESS_NOT_OK
}

/// `_downmixCoefficientsArePresent(hUniDrcConfig, downmixId, pIndex)`
fn downmix_coefficients_are_present(
    h_uni_drc_config: &UniDrcConfig,
    downmix_id: i32,
    p_index: &mut i32,
) -> i32 {
    *p_index = -1;

    for i in 0..h_uni_drc_config.downmix_instructions_count.min(6) as usize {
        let d = &h_uni_drc_config.downmix_instructions[i];
        if d.downmix_id as i32 == downmix_id {
            if d.downmix_coefficients_present != 0 {
                if d.target_channel_count > 8 {
                    return 0;
                }
                *p_index = i as i32;
                return 1;
            }
        }
    }

    0
}

/// `_getSignalPeakLevel(hSelProcInput, hUniDrcConfig, hLoudnessInfoSet, pInst,
/// downmixIdRequested, explicitPeakInformationPresent, signalPeakLevelOut, codecMode)`
fn get_signal_peak_level(
    h_sel_proc_input: &SelProcInput,
    h_uni_drc_config: &UniDrcConfig,
    h_loudness_info_set: &LoudnessInfoSet,
    p_inst: &DrcInstructionsUniDrc,
    downmix_id_requested: i32,
    explicit_peak_information_present: &mut i32,
    signal_peak_level_out: &mut FixpDbl, /* e = 7 */
    _codec_mode: SelProcCodecMode,
) -> DrcdecSelectionProcessReturn {
    let mut ret_val = DRCDEC_SELECTION_PROCESS_NO_ERROR;

    let album_mode = h_sel_proc_input.album_mode as i32;

    let mut signal_peak_level_tmp: FixpDbl;
    let mut signal_peak_level: FixpDbl = 0;

    let dmx_id = downmix_id_requested;

    let mut drc_set_id = p_inst.drc_set_id as i32;

    if drc_set_id < 0 {
        drc_set_id = 0;
    }

    *explicit_peak_information_present = 1;

    let lis = h_loudness_info_set;
    if true_peak_level_is_present(lis, drc_set_id, dmx_id, album_mode) != 0 {
        ret_val = get_true_peak_level(lis, drc_set_id, dmx_id, album_mode, &mut signal_peak_level);
        if ret_val != 0 {
            return ret_val;
        }
    } else if sample_peak_level_is_present(lis, drc_set_id, dmx_id, album_mode) != 0 {
        ret_val =
            get_sample_peak_level(lis, drc_set_id, dmx_id, album_mode, &mut signal_peak_level);
        if ret_val != 0 {
            return ret_val;
        }
    } else if true_peak_level_is_present(lis, 0x3F, dmx_id, album_mode) != 0 {
        ret_val = get_true_peak_level(lis, 0x3F, dmx_id, album_mode, &mut signal_peak_level);
        if ret_val != 0 {
            return ret_val;
        }
    } else if sample_peak_level_is_present(lis, 0x3F, dmx_id, album_mode) != 0 {
        ret_val = get_sample_peak_level(lis, 0x3F, dmx_id, album_mode, &mut signal_peak_level);
        if ret_val != 0 {
            return ret_val;
        }
    } else if limiter_peak_target_is_present(p_inst, drc_set_id, dmx_id) != 0 {
        ret_val = get_limiter_peak_target(p_inst, drc_set_id, dmx_id, &mut signal_peak_level);
        if ret_val != 0 {
            return ret_val;
        }
    } else if dmx_id != 0 {
        let mut downmix_instruction_index = 0;
        let mut downmix_peak_level_db: FixpDbl = 0;

        *explicit_peak_information_present = 0;

        signal_peak_level_tmp = 0;

        if downmix_coefficients_are_present(
            h_uni_drc_config,
            dmx_id,
            &mut downmix_instruction_index,
        ) != 0
        {
            let mut db_e = 0;
            /* e = 7, so it is possible to sum up up to 32 downmix coefficients (with e = 2) */
            let p_down = &h_uni_drc_config.downmix_instructions[downmix_instruction_index as usize];
            let base = h_uni_drc_config.channel_layout.base_channel_count as usize;
            let downmix_offset = get_downmix_offset(p_down, base as i32); /* e = 1 */
            let mut max_sum: FixpDbl = 0;

            // targetChannelCount <= 8 (checked above), baseChannelCount <= 8 (reader).
            for i in 0..p_down.target_channel_count as usize {
                let mut sum: FixpDbl = 0;
                for j in 0..base.min(8) {
                    let coeff = p_down.downmix_coefficient[j + i * base.min(8)];
                    sum = sum.wrapping_add(coeff >> 5);
                }
                if max_sum < sum {
                    max_sum = sum;
                }
            }

            max_sum = f_mult_div2(max_sum, downmix_offset).wrapping_shl(2);

            if max_sum == fl2fxconst_dbl(1.0 / 128.0) {
                downmix_peak_level_db = 0;
            } else {
                let db_m = lin2db(max_sum, 7, &mut db_e); /* e_maxSum = 7 */
                downmix_peak_level_db = scale_value(db_m, db_e - 7); /* e_downmixPeakLevelDB = 7 */
            }
        }

        if true_peak_level_is_present(lis, drc_set_id, 0, album_mode) != 0 {
            ret_val =
                get_true_peak_level(lis, drc_set_id, 0, album_mode, &mut signal_peak_level_tmp);
            if ret_val != 0 {
                return ret_val;
            }
        } else if sample_peak_level_is_present(lis, drc_set_id, 0, album_mode) != 0 {
            ret_val =
                get_sample_peak_level(lis, drc_set_id, 0, album_mode, &mut signal_peak_level_tmp);
            if ret_val != 0 {
                return ret_val;
            }
        } else if true_peak_level_is_present(lis, 0x3F, 0, album_mode) != 0 {
            ret_val = get_true_peak_level(lis, 0x3F, 0, album_mode, &mut signal_peak_level_tmp);
            if ret_val != 0 {
                return ret_val;
            }
        } else if sample_peak_level_is_present(lis, 0x3F, 0, album_mode) != 0 {
            ret_val = get_sample_peak_level(lis, 0x3F, 0, album_mode, &mut signal_peak_level_tmp);
            if ret_val != 0 {
                return ret_val;
            }
        } else if limiter_peak_target_is_present(p_inst, drc_set_id, 0) != 0 {
            ret_val = get_limiter_peak_target(p_inst, drc_set_id, 0, &mut signal_peak_level_tmp);
            if ret_val != 0 {
                return ret_val;
            }
        }

        signal_peak_level = signal_peak_level_tmp.wrapping_add(downmix_peak_level_db);
    } else {
        signal_peak_level = 0; /* worst case estimate */
        *explicit_peak_information_present = 0;
    }

    *signal_peak_level_out = signal_peak_level;

    ret_val
}

/// `_extractLoudnessPeakToAverageValue(loudnessInfo, dynamicRangeMeasurementType,
/// pLoudnessPeakToAverageValuePresent, pLoudnessPeakToAverageValue)`
fn extract_loudness_peak_to_average_value(
    loudness_info: &LoudnessInfo,
    dynamic_range_measurement_type: DynRangeMeasurementRequestType,
    p_loudness_peak_to_average_value_present: &mut i32,
    p_loudness_peak_to_average_value: &mut FixpDbl,
) -> DrcdecSelectionProcessReturn {
    let mut value_order_loudness = init_value_order();
    let mut value_order_peak_loudness = init_value_order();

    *p_loudness_peak_to_average_value_present = 0;

    for i in 0..(loudness_info.measurement_count as usize).min(8) {
        let p_loudness_measure = &loudness_info.loudness_measurement[i];
        let md = p_loudness_measure.method_definition as i32;
        let ms = p_loudness_measure.measurement_system as i32;

        if md == MD_PROGRAM_LOUDNESS {
            get_method_value(
                &mut value_order_loudness,
                p_loudness_measure.method_value,
                ms,
                MS_PROGRAMLOUDNESS,
            );
        }

        if (dynamic_range_measurement_type == DRMRT_SHORT_TERM_LOUDNESS_TO_AVG)
            && (md == MD_SHORT_TERM_LOUDNESS_MAX)
        {
            get_method_value(
                &mut value_order_peak_loudness,
                p_loudness_measure.method_value,
                ms,
                MS_PEAKLOUDNESS,
            );
        }

        if (dynamic_range_measurement_type == DRMRT_MOMENTARY_LOUDNESS_TO_AVG)
            && (md == MD_MOMENTARY_LOUDNESS_MAX)
        {
            get_method_value(
                &mut value_order_peak_loudness,
                p_loudness_measure.method_value,
                ms,
                MS_PEAKLOUDNESS,
            );
        }

        if (dynamic_range_measurement_type == DRMRT_TOP_OF_LOUDNESS_RANGE_TO_AVG)
            && (md == MD_MAX_OF_LOUDNESS_RANGE)
        {
            get_method_value(
                &mut value_order_peak_loudness,
                p_loudness_measure.method_value,
                ms,
                MS_PEAKLOUDNESS,
            );
        }
    }

    if (value_order_loudness.order > -1) && (value_order_peak_loudness.order > -1) {
        *p_loudness_peak_to_average_value =
            value_order_peak_loudness.value.wrapping_sub(value_order_loudness.value);
        *p_loudness_peak_to_average_value_present = 1;
    }

    DRCDEC_SELECTION_PROCESS_NO_ERROR
}

/*******************************************/

/// `_selectAlbumLoudness(hLoudnessInfoSet, pCandidatesPotential, pCandidatesSelected)`
fn select_album_loudness(
    h_uni_drc_config: &UniDrcConfig,
    h_loudness_info_set: &LoudnessInfoSet,
    sd: &mut [DrcdecSelection; 2],
    l: Lists,
) -> DrcdecSelectionProcessReturn {
    for i in 0..selection_get_number(&sd[l.pot]) {
        let Some(p_candidate) = selection_get_at(&sd[l.pot], i) else {
            return DRCDEC_SELECTION_PROCESS_NOT_OK;
        };

        let drc_set_id = h_uni_drc_config.drc_instructions_uni_drc[p_candidate.inst()].drc_set_id;
        for j in 0..h_loudness_info_set.loudness_info_album_count.min(12) as usize {
            if drc_set_id == h_loudness_info_set.loudness_info_album[j].drc_set_id {
                if !selection_add(&mut sd[l.sel], &p_candidate) {
                    return DRCDEC_SELECTION_PROCESS_NOT_OK;
                }
            }
        }
    }

    DRCDEC_SELECTION_PROCESS_NO_ERROR
}

/*******************************************/

/// `_findMethodDefinition(pLoudnessInfo, methodDefinition, startIndex)`
fn find_method_definition(
    p_loudness_info: &LoudnessInfo,
    method_definition: i32,
    start_index: i32,
) -> i32 {
    let mut index = -1;

    let mut i = start_index;
    while i < (p_loudness_info.measurement_count as i32).min(8) {
        if p_loudness_info.loudness_measurement[i as usize].method_definition as i32
            == method_definition
        {
            index = i;
            break;
        }
        i += 1;
    }

    index
}

/*******************************************/

/// `_getMixingLevel(hLoudnessInfoSet, downmixIdRequested, drcSetIdRequested, albumMode,
/// pMixingLevel)`
fn get_mixing_level(
    h_loudness_info_set: &LoudnessInfoSet,
    downmix_id_requested: i32,
    mut drc_set_id_requested: i32,
    album_mode: i32,
    p_mixing_level: &mut FixpDbl,
) -> DrcdecSelectionProcessReturn {
    let mixing_level_default: FixpDbl = fl2fxconst_dbl((85.0f32 / 128.0f32) as f64);

    *p_mixing_level = mixing_level_default;

    if drc_set_id_requested < 0 {
        drc_set_id_requested = 0;
    }

    for li in loudness_infos(h_loudness_info_set, album_mode) {
        if (drc_set_id_requested == li.drc_set_id as i32)
            && ((downmix_id_requested == li.downmix_id as i32)
                || (DOWNMIX_ID_ANY_DOWNMIX == li.downmix_id as i32))
        {
            let index = find_method_definition(li, MD_MIXING_LEVEL, 0);

            if index >= 0 {
                *p_mixing_level = li.loudness_measurement[index as usize].method_value;
                break;
            }
        }
    }

    DRCDEC_SELECTION_PROCESS_NO_ERROR
}
