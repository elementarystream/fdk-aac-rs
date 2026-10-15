// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.

//! Safe Rust API of the AAC decoder and encoder (PORTING.md, "Safe API").
//!
//! The decoder and the encoder are behind the `decoder` and `encoder` features, both enabled by
//! default.

pub use fdk_aac_rs_fdk::{Arch, ARCH};

#[cfg(feature = "decoder")]
pub mod chunked;
#[cfg(feature = "decoder")]
pub mod decoder;
#[cfg(feature = "encoder")]
pub mod encoder;

#[cfg(feature = "decoder")]
pub use decoder::Decoder;
#[cfg(feature = "encoder")]
pub use encoder::Encoder;
