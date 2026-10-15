// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libSBRenc/src/ps_const.h.

//! Port of `ps_const.h`: the parametric stereo constants.

/// `MAX_PS_CHANNELS`
pub const MAX_PS_CHANNELS: usize = 2;
/// `HYBRID_MAX_QMF_BANDS`
pub const HYBRID_MAX_QMF_BANDS: usize = 3;
/// `HYBRID_FILTER_LENGTH`
pub const HYBRID_FILTER_LENGTH: usize = 13;
/// `HYBRID_FILTER_DELAY`
pub const HYBRID_FILTER_DELAY: usize = (HYBRID_FILTER_LENGTH - 1) / 2;

/// `HYBRID_FRAMESIZE`
pub const HYBRID_FRAMESIZE: usize = 32;
/// `HYBRID_READ_OFFSET`
pub const HYBRID_READ_OFFSET: usize = 10;

/// `MAX_HYBRID_BANDS`
pub const MAX_HYBRID_BANDS: usize = 64 - HYBRID_MAX_QMF_BANDS + 10;

/// `PS_RESOLUTION`
pub type PsResolution = i32;
/// `PS_RES_COARSE`
pub const PS_RES_COARSE: PsResolution = 0;
/// `PS_RES_MID`
pub const PS_RES_MID: PsResolution = 1;
/// `PS_RES_FINE`
pub const PS_RES_FINE: PsResolution = 2;

/// `PS_BANDS`
pub type PsBands = i32;
/// `PS_BANDS_COARSE`
pub const PS_BANDS_COARSE: PsBands = 10;
/// `PS_BANDS_MID`
pub const PS_BANDS_MID: PsBands = 20;
/// `PS_MAX_BANDS`
pub const PS_MAX_BANDS: usize = PS_BANDS_MID as usize;

/// `PS_IID_RESOLUTION`
pub type PsIidResolution = i32;
/// `PS_IID_RES_COARSE`
pub const PS_IID_RES_COARSE: PsIidResolution = 0;
/// `PS_IID_RES_FINE`
pub const PS_IID_RES_FINE: PsIidResolution = 1;

/// `PS_ICC_ROTATION_MODE`
pub type PsIccRotationMode = i32;
/// `PS_ICC_ROT_A`
pub const PS_ICC_ROT_A: PsIccRotationMode = 0;
/// `PS_ICC_ROT_B`
pub const PS_ICC_ROT_B: PsIccRotationMode = 1;

/// `PS_DELTA`
pub type PsDelta = i32;
/// `PS_DELTA_FREQ`
pub const PS_DELTA_FREQ: PsDelta = 0;
/// `PS_DELTA_TIME`
pub const PS_DELTA_TIME: PsDelta = 1;

/// `PS_MAX_ENVELOPES`
pub const PS_MAX_ENVELOPES: usize = 4;

/// `FDK_PSENC_ERROR`
pub type FdkPsencError = i32;
/// `PSENC_OK`: No error happened. All fine.
pub const PSENC_OK: FdkPsencError = 0x0000;
/// `PSENC_INVALID_HANDLE`: Handle passed to function call was invalid.
pub const PSENC_INVALID_HANDLE: FdkPsencError = 0x0020;
/// `PSENC_MEMORY_ERROR`: Memory allocation failed.
pub const PSENC_MEMORY_ERROR: FdkPsencError = 0x0021;
/// `PSENC_INIT_ERROR`: General initialization error.
pub const PSENC_INIT_ERROR: FdkPsencError = 0x0040;
/// `PSENC_ENCODE_ERROR`: The encoding process was interrupted by an unexpected error.
pub const PSENC_ENCODE_ERROR: FdkPsencError = 0x0060;
