// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libDRCdec/src/drcDecoder.h.

//! Port of `drcDecoder.h`: definitions used in all submodules of the DRC decoder.

use fdk_aac_rs_fdk::common_fix::{fl2fxconst_dbl, fl2fxconst_sgl, FixpDbl, FixpSgl};

/// `MAX_ACTIVE_DRCS`
pub const MAX_ACTIVE_DRCS: usize = 3;

/// `DELAY_MODE`
pub type DelayMode = i32;
/// `DM_REGULAR_DELAY`
pub const DM_REGULAR_DELAY: DelayMode = 0;
/// `DM_LOW_DELAY`
pub const DM_LOW_DELAY: DelayMode = 1;

/// `DRC_ERROR`
pub type DrcError = i32;
/// `DE_OK`
pub const DE_OK: DrcError = 0;
/// `DE_NOT_OK`
pub const DE_NOT_OK: DrcError = -100;
/// `DE_PARAM_OUT_OF_RANGE`
pub const DE_PARAM_OUT_OF_RANGE: DrcError = -99;
/// `DE_PARAM_INVALID`
pub const DE_PARAM_INVALID: DrcError = -98;
/// `DE_MEMORY_ERROR`
pub const DE_MEMORY_ERROR: DrcError = -97;

/// `SUBBAND_DOMAIN_MODE`
pub type SubbandDomainMode = i32;
/// `SDM_OFF`
pub const SDM_OFF: SubbandDomainMode = 0;
/// `SDM_QMF64`
pub const SDM_QMF64: SubbandDomainMode = 1;
/// `SDM_QMF71`
pub const SDM_QMF71: SubbandDomainMode = 2;
/// `SDM_STFT256`
pub const SDM_STFT256: SubbandDomainMode = 3;

/// `DOWNMIX_ID_BASE_LAYOUT`
pub const DOWNMIX_ID_BASE_LAYOUT: i32 = 0x0;
/// `DOWNMIX_ID_ANY_DOWNMIX`
pub const DOWNMIX_ID_ANY_DOWNMIX: i32 = 0x7F;
/// `DRC_SET_ID_NO_DRC`
pub const DRC_SET_ID_NO_DRC: i32 = 0x0;
/// `DRC_SET_ID_ANY_DRC`
pub const DRC_SET_ID_ANY_DRC: i32 = 0x3F;

/// `LOCATION_MP4_INSTREAM_UNIDRC`
pub const LOCATION_MP4_INSTREAM_UNIDRC: i32 = 0x1;
/// `LOCATION_MP4_DYN_RANGE_INFO`
pub const LOCATION_MP4_DYN_RANGE_INFO: i32 = 0x2;
/// `LOCATION_MP4_COMPRESSION_VALUE`
pub const LOCATION_MP4_COMPRESSION_VALUE: i32 = 0x3;
/// `LOCATION_SELECTED`: set to location selected by system
pub const LOCATION_SELECTED: i32 = LOCATION_MP4_INSTREAM_UNIDRC;

/// `MAX_REQUESTS_DOWNMIX_ID`
pub const MAX_REQUESTS_DOWNMIX_ID: usize = 15;
/// `MAX_REQUESTS_DRC_FEATURE`
pub const MAX_REQUESTS_DRC_FEATURE: usize = 7;
/// `MAX_REQUESTS_DRC_EFFECT_TYPE`
pub const MAX_REQUESTS_DRC_EFFECT_TYPE: usize = 15;

/// `DEFAULT_LOUDNESS_DEVIATION_MAX`
pub const DEFAULT_LOUDNESS_DEVIATION_MAX: u8 = 63;

/// `DRC_INPUT_LOUDNESS_TARGET`: `FL2FXCONST_DBL(-31.0f / (float)(1 << 7))`
pub const DRC_INPUT_LOUDNESS_TARGET: FixpDbl = fl2fxconst_dbl((-31.0f32 / 128.0f32) as f64);
/// `DRC_INPUT_LOUDNESS_TARGET_SGL`: `FL2FXCONST_SGL(-31.0f / (float)(1 << 7))`
pub const DRC_INPUT_LOUDNESS_TARGET_SGL: FixpSgl = fl2fxconst_sgl((-31.0f32 / 128.0f32) as f64);
