// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libSBRenc/src/sbr_def.h.

//! Port of `sbr_def.h`: the main definitions of the SBR encoder.

use fdk_aac_rs_fdk::common_fix::{fl2fxconst_dbl, FixpDbl};

/// `noError`
pub const NO_ERROR: i32 = 0;

/// `TRUE`
pub const TRUE: i32 = 1;
/// `FALSE`
pub const FALSE: i32 = 0;

/// `RELAXATION_FLOAT`
pub const RELAXATION_FLOAT: f32 = 1e-6;
/// `RELAXATION`: `FL2FXCONST_DBL(RELAXATION_FLOAT)`
pub const RELAXATION: FixpDbl = fl2fxconst_dbl(RELAXATION_FLOAT as f64);
/// `RELAXATION_FRACT`: fractional part of RELAXATION
pub const RELAXATION_FRACT: FixpDbl = fl2fxconst_dbl(0.524288f32 as f64);
/// `RELAXATION_SHIFT`
pub const RELAXATION_SHIFT: i32 = 19;
/// `RELAXATION_LD64`: ld64(RELAXATION)
pub const RELAXATION_LD64: FixpDbl = fl2fxconst_dbl(0.31143075889f32 as f64);

/// `SBR_COMP_MODE_DELTA`
pub const SBR_COMP_MODE_DELTA: i32 = 0;
/// `SBR_COMP_MODE_CTS`
pub const SBR_COMP_MODE_CTS: i32 = 1;
/// `SBR_MAX_ENERGY_VALUES`
pub const SBR_MAX_ENERGY_VALUES: usize = 5;
/// `SBR_GLOBAL_TONALITY_VALUES`
pub const SBR_GLOBAL_TONALITY_VALUES: usize = 2;

/// `MAX_NUM_CHANNELS`
pub const MAX_NUM_CHANNELS: usize = 2;

/// `MAX_NOISE_ENVELOPES`
pub const MAX_NOISE_ENVELOPES: usize = 2;
/// `MAX_NUM_NOISE_COEFFS`
pub const MAX_NUM_NOISE_COEFFS: usize = 5;
/// `MAX_NUM_NOISE_VALUES`
pub const MAX_NUM_NOISE_VALUES: usize = MAX_NUM_NOISE_COEFFS * MAX_NOISE_ENVELOPES;

/// `MAX_ENVELOPES`
pub const MAX_ENVELOPES: usize = 5;
/// `MAX_FREQ_COEFFS`
pub const MAX_FREQ_COEFFS: usize = 48;
/// `MAX_NUM_ENVELOPE_VALUES`
pub const MAX_NUM_ENVELOPE_VALUES: usize = MAX_ENVELOPES * MAX_FREQ_COEFFS;

/// `MAX_FREQ_COEFFS_FS44100`
pub const MAX_FREQ_COEFFS_FS44100: i32 = 35;
/// `MAX_FREQ_COEFFS_FS48000`
pub const MAX_FREQ_COEFFS_FS48000: i32 = 32;

/// `NO_OF_ESTIMATES_LC`
pub const NO_OF_ESTIMATES_LC: i32 = 4;
/// `NO_OF_ESTIMATES_LD`
pub const NO_OF_ESTIMATES_LD: i32 = 3;
/// `MAX_NO_OF_ESTIMATES`
pub const MAX_NO_OF_ESTIMATES: usize = 4;

/// `NOISE_FLOOR_OFFSET`
pub const NOISE_FLOOR_OFFSET: i32 = 6;
/// `NOISE_FLOOR_OFFSET_64`
pub const NOISE_FLOOR_OFFSET_64: FixpDbl = fl2fxconst_dbl(0.09375f32 as f64);

/// `LOW_RES`
pub const LOW_RES: usize = 0;
/// `HIGH_RES`
pub const HIGH_RES: usize = 1;

/// `LO`
pub const LO: usize = 0;
/// `HI`
pub const HI: usize = 1;

/// `LENGTH_SBR_FRAME_INFO`
pub const LENGTH_SBR_FRAME_INFO: i32 = 35;

/// `SBR_NSFB_LOW_RES`
pub const SBR_NSFB_LOW_RES: i32 = 9;
/// `SBR_NSFB_HIGH_RES`
pub const SBR_NSFB_HIGH_RES: i32 = 18;

/// `SBR_XPOS_CTRL_DEFAULT`
pub const SBR_XPOS_CTRL_DEFAULT: i32 = 2;

/// `SBR_FREQ_SCALE_DEFAULT`
pub const SBR_FREQ_SCALE_DEFAULT: i32 = 2;
/// `SBR_ALTER_SCALE_DEFAULT`
pub const SBR_ALTER_SCALE_DEFAULT: i32 = 1;
/// `SBR_NOISE_BANDS_DEFAULT`
pub const SBR_NOISE_BANDS_DEFAULT: i32 = 2;

/// `SBR_LIMITER_BANDS_DEFAULT`
pub const SBR_LIMITER_BANDS_DEFAULT: i32 = 2;
/// `SBR_LIMITER_GAINS_DEFAULT`
pub const SBR_LIMITER_GAINS_DEFAULT: i32 = 2;
/// `SBR_LIMITER_GAINS_INFINITE`
pub const SBR_LIMITER_GAINS_INFINITE: i32 = 3;
/// `SBR_INTERPOL_FREQ_DEFAULT`
pub const SBR_INTERPOL_FREQ_DEFAULT: i32 = 1;
/// `SBR_SMOOTHING_LENGTH_DEFAULT`
pub const SBR_SMOOTHING_LENGTH_DEFAULT: i32 = 0;

// sbr_header
/// `SI_SBR_AMP_RES_BITS`
pub const SI_SBR_AMP_RES_BITS: u32 = 1;
/// `SI_SBR_COUPLING_BITS`
pub const SI_SBR_COUPLING_BITS: u32 = 1;
/// `SI_SBR_START_FREQ_BITS`
pub const SI_SBR_START_FREQ_BITS: u32 = 4;
/// `SI_SBR_STOP_FREQ_BITS`
pub const SI_SBR_STOP_FREQ_BITS: u32 = 4;
/// `SI_SBR_XOVER_BAND_BITS`
pub const SI_SBR_XOVER_BAND_BITS: u32 = 3;
/// `SI_SBR_RESERVED_BITS`
pub const SI_SBR_RESERVED_BITS: u32 = 2;
/// `SI_SBR_DATA_EXTRA_BITS`
pub const SI_SBR_DATA_EXTRA_BITS: u32 = 1;
/// `SI_SBR_HEADER_EXTRA_1_BITS`
pub const SI_SBR_HEADER_EXTRA_1_BITS: u32 = 1;
/// `SI_SBR_HEADER_EXTRA_2_BITS`
pub const SI_SBR_HEADER_EXTRA_2_BITS: u32 = 1;

// sbr_header extra 1
/// `SI_SBR_FREQ_SCALE_BITS`
pub const SI_SBR_FREQ_SCALE_BITS: u32 = 2;
/// `SI_SBR_ALTER_SCALE_BITS`
pub const SI_SBR_ALTER_SCALE_BITS: u32 = 1;
/// `SI_SBR_NOISE_BANDS_BITS`
pub const SI_SBR_NOISE_BANDS_BITS: u32 = 2;

// sbr_header extra 2
/// `SI_SBR_LIMITER_BANDS_BITS`
pub const SI_SBR_LIMITER_BANDS_BITS: u32 = 2;
/// `SI_SBR_LIMITER_GAINS_BITS`
pub const SI_SBR_LIMITER_GAINS_BITS: u32 = 2;
/// `SI_SBR_INTERPOL_FREQ_BITS`
pub const SI_SBR_INTERPOL_FREQ_BITS: u32 = 1;
/// `SI_SBR_SMOOTHING_LENGTH_BITS`
pub const SI_SBR_SMOOTHING_LENGTH_BITS: u32 = 1;

// sbr_grid
/// `SBR_CLA_BITS`: size of bs_frame_class
pub const SBR_CLA_BITS: u32 = 2;
/// `SBR_CLA_BITS_LD`: size of bs_frame_class
pub const SBR_CLA_BITS_LD: u32 = 1;
/// `SBR_ENV_BITS`: size of bs_num_env_raw
pub const SBR_ENV_BITS: u32 = 2;
/// `SBR_ABS_BITS`: size of bs_abs_bord_raw for HE-AAC
pub const SBR_ABS_BITS: u32 = 2;
/// `SBR_NUM_BITS`: size of bs_num_rel
pub const SBR_NUM_BITS: u32 = 2;
/// `SBR_REL_BITS`: size of bs_rel_bord_raw
pub const SBR_REL_BITS: u32 = 2;
/// `SBR_RES_BITS`: size of bs_freq_res_flag
pub const SBR_RES_BITS: u32 = 1;
/// `SBR_DIR_BITS`: size of bs_df_flag
pub const SBR_DIR_BITS: u32 = 1;

// sbr_data
/// `SI_SBR_INVF_MODE_BITS`
pub const SI_SBR_INVF_MODE_BITS: u32 = 2;

/// `SI_SBR_START_ENV_BITS_AMP_RES_3_0`
pub const SI_SBR_START_ENV_BITS_AMP_RES_3_0: i32 = 6;
/// `SI_SBR_START_ENV_BITS_BALANCE_AMP_RES_3_0`
pub const SI_SBR_START_ENV_BITS_BALANCE_AMP_RES_3_0: i32 = 5;
/// `SI_SBR_START_NOISE_BITS_AMP_RES_3_0`
pub const SI_SBR_START_NOISE_BITS_AMP_RES_3_0: i32 = 5;
/// `SI_SBR_START_NOISE_BITS_BALANCE_AMP_RES_3_0`
pub const SI_SBR_START_NOISE_BITS_BALANCE_AMP_RES_3_0: i32 = 5;

/// `SI_SBR_START_ENV_BITS_AMP_RES_1_5`
pub const SI_SBR_START_ENV_BITS_AMP_RES_1_5: i32 = 7;
/// `SI_SBR_START_ENV_BITS_BALANCE_AMP_RES_1_5`
pub const SI_SBR_START_ENV_BITS_BALANCE_AMP_RES_1_5: i32 = 6;

/// `SI_SBR_EXTENDED_DATA_BITS`
pub const SI_SBR_EXTENDED_DATA_BITS: u32 = 1;
/// `SI_SBR_EXTENSION_SIZE_BITS`
pub const SI_SBR_EXTENSION_SIZE_BITS: u32 = 4;
/// `SI_SBR_EXTENSION_ESC_COUNT_BITS`
pub const SI_SBR_EXTENSION_ESC_COUNT_BITS: u32 = 8;
/// `SI_SBR_EXTENSION_ID_BITS`
pub const SI_SBR_EXTENSION_ID_BITS: u32 = 2;

/// `SBR_EXTENDED_DATA_MAX_CNT`
pub const SBR_EXTENDED_DATA_MAX_CNT: usize = 15 + 255;

/// `EXTENSION_ID_PS_CODING`
pub const EXTENSION_ID_PS_CODING: u32 = 2;

// Envelope coding constants
/// `FREQ`
pub const FREQ: i32 = 0;
/// `TIME`
pub const TIME: i32 = 1;

/// `QMF_SCALE_OFFSET`: qmf data scaling
pub const QMF_SCALE_OFFSET: i32 = 7;

// huffman tables
/// `CODE_BOOK_SCF_LAV00`
pub const CODE_BOOK_SCF_LAV00: i32 = 60;
/// `CODE_BOOK_SCF_LAV01`
pub const CODE_BOOK_SCF_LAV01: i32 = 31;
/// `CODE_BOOK_SCF_LAV10`
pub const CODE_BOOK_SCF_LAV10: i32 = 60;
/// `CODE_BOOK_SCF_LAV11`
pub const CODE_BOOK_SCF_LAV11: i32 = 31;
/// `CODE_BOOK_SCF_LAV_BALANCE11`
pub const CODE_BOOK_SCF_LAV_BALANCE11: i32 = 12;
/// `CODE_BOOK_SCF_LAV_BALANCE10`
pub const CODE_BOOK_SCF_LAV_BALANCE10: i32 = 24;

/// `AMP_RES`
pub type AmpRes = i32;
/// `SBR_AMP_RES_1_5`
pub const SBR_AMP_RES_1_5: AmpRes = 0;
/// `SBR_AMP_RES_3_0`
pub const SBR_AMP_RES_3_0: AmpRes = 1;

/// `XPOS_MODE`
pub type XposMode = i32;
/// `XPOS_MDCT`
pub const XPOS_MDCT: XposMode = 0;
/// `XPOS_MDCT_CROSS`
pub const XPOS_MDCT_CROSS: XposMode = 1;
/// `XPOS_LC`
pub const XPOS_LC: XposMode = 2;
/// `XPOS_RESERVED`
pub const XPOS_RESERVED: XposMode = 3;
/// `XPOS_SWITCHED`: not a real choice but used here to control behaviour
pub const XPOS_SWITCHED: XposMode = 4;

/// `INVF_MODE`
pub type InvfMode = i32;
/// `INVF_OFF`
pub const INVF_OFF: InvfMode = 0;
/// `INVF_LOW_LEVEL`
pub const INVF_LOW_LEVEL: InvfMode = 1;
/// `INVF_MID_LEVEL`
pub const INVF_MID_LEVEL: InvfMode = 2;
/// `INVF_HIGH_LEVEL`
pub const INVF_HIGH_LEVEL: InvfMode = 3;
/// `INVF_SWITCHED`: not a real choice but used here to control behaviour
pub const INVF_SWITCHED: InvfMode = 4;
