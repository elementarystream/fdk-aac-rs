// Smoke tests of the cref infrastructure: the reference links, its API is renamed to cref_*, and
// the reference variant matches the variant of the port.

#![cfg(not(cref_missing))]
// Test tables of tuples are clearer inline than behind type aliases.
#![allow(clippy::type_complexity)]

use fdk_aac_rs_cref::infra::*;
use fdk_aac_rs_fdk::{Arch, ARCH};

#[test]
fn reference_links_and_opens_a_decoder() {
    // TT_MP4_ADTS = 2
    assert_eq!(unsafe { cref_infra_open_close(2) }, 0);
}

#[test]
fn reference_variant_matches_port_variant() {
    // fMult(MINVAL_DBL, MINVAL_DBL) saturates nowhere; 0x40000000^2 shows the rounding: the
    // AArch64 variant computes (a*b)>>31, the x86 variant ((a*b)>>32)<<1.
    let (a, b) = (0x4000_0001i32, 0x0000_0003i32);
    let r = unsafe { cref_infra_fmult(a, b) };
    let p = a as i64 * b as i64;
    let expected = match ARCH {
        Arch::Aarch64 => (p >> 31) as i32,
        Arch::X86 | Arch::X86_32 | Arch::Generic => ((p >> 32) << 1) as i32,
    };
    assert_eq!(r, expected, "ARCH = {ARCH:?}");
}
