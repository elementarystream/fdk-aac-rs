// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.

//! Fixed-point DSP, bitstream and system primitives (port of libFDK and libSYS).
//!
//! One module per C++ source pair (`foo.h` + `foo.cpp`, plus its `arm/` or `x86/`
//! variant), see PORTING.md, "Workspace".
//!
//! This crate is an internal part of [fdk-aac-rs](https://crates.io/crates/fdk-aac-rs): use the
//! `fdk-aac-rs` crate (the safe API) or `fdk-aac-rs-capi` (the C ABI). Its items mirror the C++
//! library and have no stability guarantee across versions; functions other than the library
//! entry points expect the arguments their C++ callers pass and may panic on others.

// The SSE2 kernels are used on every x86 target without a run-time check.
#[cfg(all(target_arch = "x86", not(target_feature = "sse2")))]
compile_error!("fdk-aac-rs needs SSE2 on 32-bit x86 (the i686 targets have it, i586 targets not)");

pub mod arch;
#[macro_use]
pub mod fdk_assert;
pub use arch::{Arch, ARCH};

pub mod abs;
pub mod archdef;
pub mod audio;
pub mod autocorr2nd;
pub mod bitbuffer;
pub mod bitstream;
pub mod clz;
pub mod common_fix;
pub mod core;
pub mod cplx_mul;
pub mod crc;
pub mod dct;
pub mod decorrelate;
pub mod digest;
pub mod fft;
pub mod fft_rad2;
pub mod fixmadd;
pub mod fixminmax;
pub mod fixmul;
pub mod fixpoint_math;
pub mod generic_stds;
pub mod huff_nodes;
pub mod hybrid;
pub mod lpc;
pub mod machine_type;
pub mod matrix_calloc;
pub mod mdct;
pub mod nlc_dec;
pub mod par;
pub mod qmf;
pub mod qmf_domain;
pub mod qmf_pcm;
pub mod scale;
pub mod scramble;
pub mod simd;
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
pub mod simd_x86;
pub mod syslib_channel_map_descr;
pub mod tools_rom;
pub mod trig_fcts;
pub mod x87;
