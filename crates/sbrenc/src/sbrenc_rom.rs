// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libSBRenc/src/sbrenc_rom.h, libSBRenc/src/sbrenc_rom.cpp.

//! Port of `sbrenc_rom.h` and `sbrenc_rom.cpp`: the Huffman tables of the envelope and noise
//! floor coding and the SBR and PS tuning tables.
//!
//! The tables are generated from the C++ source by `gen_tables.py` (in this module's directory),
//! which also generates the file-static tables of the other modules (`<module>/tables.rs`).

mod tables;
pub use tables::*;
