// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.

//! MPEG Surround encoder (port of libSACenc).
//!
//! One module per C++ source pair (`foo.h` + `foo.cpp`, plus its `arm/` or `x86/`
//! variant), see PORTING.md, "Workspace".
//!
//! This crate is an internal part of [fdk-aac-rs](https://crates.io/crates/fdk-aac-rs): use the
//! `fdk-aac-rs` crate (the safe API) or `fdk-aac-rs-capi` (the C ABI). Its items mirror the C++
//! library and have no stability guarantee across versions; functions other than the library
//! entry points expect the arguments their C++ callers pass and may panic on others.

pub mod sacenc_bitstream;
pub mod sacenc_const;
pub mod sacenc_delay;
pub mod sacenc_dmx_tdom_enh;
pub mod sacenc_filter;
pub mod sacenc_framewindowing;
pub mod sacenc_huff_tab;
pub mod sacenc_lib;
pub mod sacenc_nlc_enc;
pub mod sacenc_onsetdetect;
pub mod sacenc_paramextract;
pub mod sacenc_staticgain;
pub mod sacenc_tree;
pub mod sacenc_vectorfunctions;
