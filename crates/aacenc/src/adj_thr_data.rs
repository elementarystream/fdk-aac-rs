// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACenc/src/adj_thr_data.h.

//! Port of `adj_thr_data.h`: data of the threshold adjustment.

use fdk_aac_rs_fdk::common_fix::FixpDbl;

/// `AACENC_BIT_DISTRIBUTION_MODE`
pub type AacencBitDistributionMode = i32;
/// `AACENC_BD_MODE_INTER_ELEMENT`
pub const AACENC_BD_MODE_INTER_ELEMENT: AacencBitDistributionMode = 0;
/// `AACENC_BD_MODE_INTRA_ELEMENT`
pub const AACENC_BD_MODE_INTRA_ELEMENT: AacencBitDistributionMode = 1;

/// `AACENC_BITRES_MODE`
pub type AacencBitresMode = i32;
/// `AACENC_BR_MODE_FULL`
pub const AACENC_BR_MODE_FULL: AacencBitresMode = 0;
/// `AACENC_BR_MODE_REDUCED`
pub const AACENC_BR_MODE_REDUCED: AacencBitresMode = 1;
/// `AACENC_BR_MODE_DISABLED`
pub const AACENC_BR_MODE_DISABLED: AacencBitresMode = 2;

/// `BRES_PARAM` (layout identical to the C struct).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BresParam {
    pub clip_save_low: FixpDbl,
    pub clip_save_high: FixpDbl,
    pub min_bit_save: FixpDbl,
    pub max_bit_save: FixpDbl,
    pub clip_spend_low: FixpDbl,
    pub clip_spend_high: FixpDbl,
    pub min_bit_spend: FixpDbl,
    pub max_bit_spend: FixpDbl,
}

/// `AH_PARAM` (layout identical to the C struct).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AhParam {
    pub modify_min_snr: i32,
    pub start_sfb_l: i32,
    pub start_sfb_s: i32,
}

/// `MINSNR_ADAPT_PARAM` (layout identical to the C struct).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MinsnrAdaptParam {
    pub max_red: FixpDbl,
    pub start_ratio: FixpDbl,
    pub max_ratio: FixpDbl,
    pub red_ratio_fac: FixpDbl,
    pub red_offs: FixpDbl,
}

/// `ATS_ELEMENT` (layout identical to the C struct).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AtsElement {
    /// parameters for bitreservoir control
    pub pe_min: i32,
    pub pe_max: i32,
    /// constant offset to pe
    pub pe_offset: i32,
    /// constant PeFactor
    pub bits2_pe_factor_m: FixpDbl,
    pub bits2_pe_factor_e: i32,
    /// avoid hole parameters
    pub ah_param: AhParam,
    /// parameters for adaptation of minSnr
    pub min_snr_adapt_param: MinsnrAdaptParam,
    /// values for correction of pe
    pub pe_last: i32,
    pub dyn_bits_last: i32,
    pub pe_correction_factor_m: FixpDbl,
    pub pe_correction_factor_e: i32,
    /// vbr encoding
    pub vbr_qual_factor: FixpDbl,
    pub chaos_measure_old: FixpDbl,
    /// threshold weighting
    pub chaos_measure_en_fac: [FixpDbl; 2],
    pub last_en_fac_patch: [i32; 2],
}

/// `ADJ_THR_STATE`
///
/// `adjThrStateElem[]` holds pointers to separately allocated `ATS_ELEMENT`s in C; here they are
/// owned (`nElements` entries).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AdjThrState {
    pub bres_param_long: BresParam,
    pub bres_param_short: BresParam,
    pub adj_thr_state_elem: Vec<AtsElement>,
    pub bit_distribution_mode: AacencBitDistributionMode,
    pub max_iter_2nd_guess: i32,
}
