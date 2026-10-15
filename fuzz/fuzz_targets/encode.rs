#![no_main]
//! The encoder with random parameter sequences and PCM (src/enc.rs).
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| fdk_aac_rs_fuzz::enc::fuzz(data));
