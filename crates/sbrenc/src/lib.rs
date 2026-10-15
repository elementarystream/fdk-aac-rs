// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.

//! SBR and PS encoder (port of libSBRenc).
//!
//! One module per C++ source pair (`foo.h` + `foo.cpp`, plus its `arm/` or `x86/`
//! variant), see PORTING.md, "Workspace".
//!
//! This crate is an internal part of [fdk-aac-rs](https://crates.io/crates/fdk-aac-rs): use the
//! `fdk-aac-rs` crate (the safe API) or `fdk-aac-rs-capi` (the C ABI). Its items mirror the C++
//! library and have no stability guarantee across versions; functions other than the library
//! entry points expect the arguments their C++ callers pass and may panic on others.

pub mod bit_sbr;
pub mod cmondata;
pub mod code_env;
pub mod env_bit;
pub mod env_est;
pub mod fram_gen;
pub mod invf_est;
pub mod mh_det;
pub mod nf_est;
pub mod ps_bitenc;
pub mod ps_const;
pub mod ps_encode;
pub mod ps_main;
pub mod resampler;
pub mod sbr;
pub mod sbr_def;
pub mod sbr_encoder;
pub mod sbr_misc;
pub mod sbrenc_freq_sca;
pub mod sbrenc_ram;
pub mod sbrenc_rom;
mod simd;
pub mod ton_corr;
pub mod tran_det;
