// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.

//! SBR and PS decoder (port of libSBRdec).
//!
//! One module per C++ source pair (`foo.h` + `foo.cpp`, plus its `arm/` or `x86/`
//! variant), see PORTING.md, "Workspace".
//!
//! This crate is an internal part of [fdk-aac-rs](https://crates.io/crates/fdk-aac-rs): use the
//! `fdk-aac-rs` crate (the safe API) or `fdk-aac-rs-capi` (the C ABI). Its items mirror the C++
//! library and have no stability guarantee across versions; functions other than the library
//! entry points expect the arguments their C++ callers pass and may panic on others.

mod digest;
pub mod env_calc;
pub mod env_dec;
pub mod env_extr;
pub mod hbe;
pub mod hfgen_pre_flat;
pub mod huff_dec;
pub mod lpp_tran;
pub mod psbitdec;
pub mod psdec;
pub mod psdec_drm;
pub mod psdecrom_drm;
pub mod pvc_dec;
pub mod sbr_deb;
pub mod sbr_dec;
pub mod sbr_ram;
pub mod sbr_rom;
pub mod sbrdec_drc;
pub mod sbrdec_freq_sca;
pub mod sbrdecoder;
pub mod transcendent;
