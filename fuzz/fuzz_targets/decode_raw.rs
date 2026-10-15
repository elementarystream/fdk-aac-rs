#![no_main]
//! The decoder through the raw transport (or LATM MCP0) with ConfigRaw (src/dec.rs).
use fdk_aac_rs_fuzz::dec::{fuzz, DecTarget};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| fuzz(DecTarget::Raw, data));
