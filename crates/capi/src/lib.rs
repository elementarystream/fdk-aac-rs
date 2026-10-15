// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.

//! C ABI that replaces `libfdk-aac` (PORTING.md, "C ABI").
//!
//! The exported symbols are those of `fdk-aac.sym` plus `aacDecoder_RawISOBMFFData`, which the
//! upstream static library exposes.

pub mod dec;
pub mod enc;
pub mod lib_info;
