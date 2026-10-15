// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libSBRdec/src/sbr_rom.h, libSBRdec/src/sbr_rom.cpp.

//! Port of `sbr_rom.h` and `sbr_rom.cpp`: the constant tables of the SBR decoder.
//!
//! The tables are generated from the C++ reference by `sbr_rom/gen_tables.py` (see
//! [`tables`]); nothing is retyped.

pub mod tables;
pub use tables::*;

/// `INV_INT_TABLE_SIZE`
pub const INV_INT_TABLE_SIZE: i32 = 49;
/// `SBR_NF_NO_RANDOM_VAL`: size of the random number array for the noise floor
pub const SBR_NF_NO_RANDOM_VAL: i32 = 512;

/// `NUM_WHFACTOR_TABLE_ENTRIES`
pub const NUM_WHFACTOR_TABLE_ENTRIES: usize = 9;

/// `INV_TABLE_BITS`
pub const INV_TABLE_BITS: i32 = 8;
/// `INV_TABLE_SIZE`
pub const INV_TABLE_SIZE: i32 = 1 << INV_TABLE_BITS;
