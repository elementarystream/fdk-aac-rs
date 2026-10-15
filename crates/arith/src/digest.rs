// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Not in C: the state digest of the arithmetic decoder (see fdk_aac_rs_fdk::digest).

//! [`StateDigest`](fdk_aac_rs_fdk::digest::StateDigest) of the arithmetic decoder context (not in
//! C; see `fdk_aac_rs_fdk::digest`).

use fdk_aac_rs_fdk::impl_state_digest;

use crate::ac_arith_coder::CArcoData;

impl_state_digest!(CArcoData { m_number_lines_prev, c_prev });
