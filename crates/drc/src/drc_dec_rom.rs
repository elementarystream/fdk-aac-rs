// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libDRCdec/src/drcDec_rom.h, libDRCdec/src/drcDec_rom.cpp.

//! Port of `drcDec_rom.h`: the constant tables of the DRC decoder.
//!
//! The tables are generated from the C++ source by `drc_dec_rom/gen_tables.py` with
//! `scripts/dump_table.py` ([`tables`]); both reference builds give the same values.

#[rustfmt::skip]
pub mod tables;

pub use tables::*;
