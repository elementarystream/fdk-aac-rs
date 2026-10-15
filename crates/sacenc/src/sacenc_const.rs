// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libSACenc/src/sacenc_const.h.

//! Port of `sacenc_const.h`: the constants of the MPEG Surround encoder.

/// `NUM_QMF_BANDS`
pub const NUM_QMF_BANDS: usize = 64;
/// `MAX_QMF_BANDS`
pub const MAX_QMF_BANDS: usize = 128;

/// `SACENC_MAX_NUM_BOXES`
pub const SACENC_MAX_NUM_BOXES: usize = 1;
/// `SACENC_MAX_INPUT_CHANNELS`
pub const SACENC_MAX_INPUT_CHANNELS: usize = 2;
/// `SACENC_MAX_OUTPUT_CHANNELS`
pub const SACENC_MAX_OUTPUT_CHANNELS: usize = 1;

/// `SACENC_FLOAT_EPSILON`: `1e-9f`
pub const SACENC_FLOAT_EPSILON: f32 = 1e-9f32;
