// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.

//! AAC encoder core and encoder API (port of libAACenc).
//!
//! One module per C++ source pair (`foo.h` + `foo.cpp`, plus its `arm/` or `x86/`
//! variant), see PORTING.md, "Workspace".
//!
//! This crate is an internal part of [fdk-aac-rs](https://crates.io/crates/fdk-aac-rs): use the
//! `fdk-aac-rs` crate (the safe API) or `fdk-aac-rs-capi` (the C ABI). Its items mirror the C++
//! library and have no stability guarantee across versions; functions other than the library
//! entry points expect the arguments their C++ callers pass and may panic on others.

pub mod aac_enc_ram;
pub mod aac_enc_rom;
pub mod aacenc;
pub mod aacenc_lib;
pub mod aacenc_pns;
pub mod aacenc_tns;
pub mod adj_thr;
pub mod adj_thr_data;
pub mod band_nrg;
pub mod bandwidth;
pub mod bit_cnt;
pub mod bitenc;
pub mod block_switch;
pub mod channel_map;
pub mod chaosmeasure;
pub mod dyn_bits;
pub mod grp_data;
pub mod intensity;
pub mod interface;
pub mod line_pe;
pub mod metadata_compressor;
pub mod metadata_main;
pub mod mps_main;
pub mod ms_stereo;
pub mod noisedet;
pub mod par;
pub mod pns_func;
pub mod pnsparam;
pub mod pre_echo_control;
pub mod psy_configuration;
pub mod psy_const;
pub mod psy_data;
pub mod psy_main;
pub mod qc_data;
pub mod qc_main;
pub mod quantize;
pub mod sf_estim;
mod simd;
pub mod spreading;
pub mod tns_func;
pub mod tonality;
pub mod transform;
