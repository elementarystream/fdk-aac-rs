// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACenc/src/tns_func.h.

//! Port of `tns_func.h`: the TNS functions, which are defined in `aacenc_tns.cpp` (ported in
//! [`crate::aacenc_tns`]).

pub use crate::aacenc_tns::{
    fdk_aac_enc_init_tns_configuration, fdk_aac_enc_tns_detect, fdk_aac_enc_tns_encode,
    fdk_aac_enc_tns_sync,
};
