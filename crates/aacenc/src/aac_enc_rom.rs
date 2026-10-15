// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACenc/src/aacEnc_rom.h, libAACenc/src/aacEnc_rom.cpp.

//! Port of `aacEnc_rom.h`: the constant tables of the AAC encoder.
//!
//! The tables are generated into [`tables`] by `aac_enc_rom/gen_tables.py` (never retyped).

pub mod tables;

pub use tables::*;

/// `MANT_DIGITS`
pub const MANT_DIGITS: i32 = 9;
/// `MANT_SIZE`
pub const MANT_SIZE: i32 = 1 << MANT_DIGITS;
