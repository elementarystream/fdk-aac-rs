// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.

//! MPEG-D DRC decoder (port of libDRCdec).
//!
//! One module per C++ source pair (`foo.h` + `foo.cpp`, plus its `arm/` or `x86/`
//! variant), see PORTING.md, "Workspace".
//!
//! This crate is an internal part of [fdk-aac-rs](https://crates.io/crates/fdk-aac-rs): use the
//! `fdk-aac-rs` crate (the safe API) or `fdk-aac-rs-capi` (the C ABI). Its items mirror the C++
//! library and have no stability guarantee across versions; functions other than the library
//! entry points expect the arguments their C++ callers pass and may panic on others.

mod digest;
pub mod drc_dec_gain_decoder;
pub mod drc_dec_lib;
pub mod drc_dec_reader;
pub mod drc_dec_rom;
pub mod drc_dec_selection_process;
pub mod drc_dec_tools;
pub mod drc_dec_types;
pub mod drc_decoder;
pub mod drc_gain_dec_init;
pub mod drc_gain_dec_preprocess;
pub mod drc_gain_dec_process;
