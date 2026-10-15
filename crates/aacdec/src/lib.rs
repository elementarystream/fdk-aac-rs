// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.

//! AAC decoder core and decoder API (port of libAACdec).
//!
//! One module per C++ source pair (`foo.h` + `foo.cpp`, plus its `arm/` or `x86/`
//! variant), see PORTING.md, "Workspace".
//!
//! This crate is an internal part of [fdk-aac-rs](https://crates.io/crates/fdk-aac-rs): use the
//! `fdk-aac-rs` crate (the safe API) or `fdk-aac-rs-capi` (the C ABI). Its items mirror the C++
//! library and have no stability guarantee across versions; functions other than the library
//! entry points expect the arguments their C++ callers pass and may panic on others.

pub mod aac_ram;
pub mod aac_rom;
pub mod aacdec_drc;
pub mod aacdec_drc_types;
pub mod aacdec_hcr;
pub mod aacdec_hcr_bit;
pub mod aacdec_hcr_types;
pub mod aacdec_hcrs;
pub mod aacdec_pns;
pub mod aacdec_tns;
pub mod aacdecoder;
pub mod aacdecoder_lib;
pub mod block;
pub mod channel;
pub mod channelinfo;
pub mod conceal;
pub mod conceal_types;
pub mod delay;
pub mod digest;
pub mod ldfiltbank;
pub mod overlapadd;
pub mod pipeline;
pub mod pulsedata;
pub mod rvlc;
pub mod rvlc_info;
pub mod rvlcbit;
pub mod rvlcconceal;
pub mod stereo;
pub mod usacdec_ace_d4t64;
pub mod usacdec_ace_ltp;
pub mod usacdec_acelp;
pub mod usacdec_const;
pub mod usacdec_fac;
pub mod usacdec_lpc;
pub mod usacdec_lpd;
pub mod usacdec_rom;
