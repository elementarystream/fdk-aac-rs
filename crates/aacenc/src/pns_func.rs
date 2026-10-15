// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libAACenc/src/pns_func.h.

//! Port of `pns_func.h`: the PNS functions, which are defined in `aacenc_pns.cpp` (ported in
//! [`crate::aacenc_pns`]).

pub use crate::aacenc_pns::{
    fdk_aac_enc_code_pns_channel, fdk_aac_enc_init_pns_configuration, fdk_aac_enc_pns_detect,
    fdk_aac_enc_post_process_pns_channel_pair, fdk_aac_enc_pre_process_pns_channel_pair,
};
