#![no_main]
//! The decoder through the LATM MCP1 transport (src/dec.rs).
use fdk_aac_rs_fuzz::dec::{fuzz, DecTarget};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| fuzz(DecTarget::Latm, data));
