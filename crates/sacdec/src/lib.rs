// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.

//! MPEG Surround decoder (port of libSACdec).
//!
//! One module per C++ source pair (`foo.h` + `foo.cpp`, plus its `arm/` or `x86/`
//! variant), see PORTING.md, "Workspace".
//!
//! This crate is an internal part of [fdk-aac-rs](https://crates.io/crates/fdk-aac-rs): use the
//! `fdk-aac-rs` crate (the safe API) or `fdk-aac-rs-capi` (the C ABI). Its items mirror the C++
//! library and have no stability guarantee across versions; functions other than the library
//! entry points expect the arguments their C++ callers pass and may panic on others.

mod digest;
pub mod sac_bitdec;
pub mod sac_calc_m1_and_m2;
pub mod sac_dec;
pub mod sac_dec_conceal;
pub mod sac_dec_errorcodes;
pub mod sac_dec_interface;
pub mod sac_dec_lib;
pub mod sac_dec_ssc_struct;
pub mod sac_process;
pub mod sac_qmf;
pub mod sac_reshape_bb_env;
pub mod sac_rom;
pub mod sac_smoothing;
pub mod sac_stp;
pub mod sac_tsd;
