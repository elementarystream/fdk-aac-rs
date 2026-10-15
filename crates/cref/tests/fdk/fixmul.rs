// Differential tests of fdk_aac_rs_fdk::fixmul against the C++ reference.

use crate::common_fix::*;
use fdk_aac_rs_cref::fdk_fixmul::*;
use fdk_aac_rs_fdk::fixmul::*;
use fdk_aac_rs_fdk::{Arch, ARCH};

#[test]
fn fixmul_DD() {
    each_dd(101, N, |a, b| {
        assert_eq!(fixmuldiv2_dd(a, b), unsafe { cref_fixmuldiv2_DD(a, b) }, "{a:#x} {b:#x}");
        assert_eq!(fixmuldiv2_bit_exact_dd(a, b), unsafe { cref_fixmuldiv2BitExact_DD(a, b) });
        assert_eq!(fixmul_dd(a, b), unsafe { cref_fixmul_DD(a, b) }, "fixmul_DD {a:#x} {b:#x}");
        assert_eq!(fixmul_bit_exact_dd(a, b), unsafe { cref_fixmulBitExact_DD(a, b) });
    });
}

/// The variant difference of `fixmul_DD`: `(a*b) >> 31` keeps bit 0 on `aarch64`.
#[test]
fn fixmul_DD_variant() {
    let (a, b) = (0x4000_0001, 3);
    let r = unsafe { cref_fixmul_DD(a, b) };
    match ARCH {
        Arch::Aarch64 => assert_eq!(r, 1),
        Arch::X86 | Arch::X86_32 | Arch::Generic => assert_eq!(r, 0),
    }
    assert_eq!(fixmul_dd(a, b), r);
}

#[test]
fn fixmul_SS() {
    each_ss(102, N, |a, b| {
        assert_eq!(fixmuldiv2_ss(a, b), unsafe { cref_fixmuldiv2_SS(a, b) });
        assert_eq!(fixmul_ss(a, b), unsafe { cref_fixmul_SS(a, b) }, "fixmul_SS {a} {b}");
    });
}

#[test]
fn fixmul_SD_DS() {
    each_sd(103, N, |a, b| {
        assert_eq!(fixmuldiv2_sd(a, b), unsafe { cref_fixmuldiv2_SD(a, b) }, "{a} {b:#x}");
        assert_eq!(fixmuldiv2_ds(b, a), unsafe { cref_fixmuldiv2_DS(b, a) });
        assert_eq!(fixmuldiv2_bit_exact_sd(a, b), unsafe { cref_fixmuldiv2BitExact_SD(a, b) });
        assert_eq!(fixmuldiv2_bit_exact_ds(b, a), unsafe { cref_fixmuldiv2BitExact_DS(b, a) });
        assert_eq!(fixmul_sd(a, b), unsafe { cref_fixmul_SD(a, b) }, "fixmul_SD {a} {b:#x}");
        assert_eq!(fixmul_ds(b, a), unsafe { cref_fixmul_DS(b, a) });
        assert_eq!(fixmul_bit_exact_sd(a, b), unsafe { cref_fixmulBitExact_SD(a, b) });
        assert_eq!(fixmul_bit_exact_ds(b, a), unsafe { cref_fixmulBitExact_DS(b, a) });
    });
}

#[test]
fn fixpow2_D_S() {
    each_d(104, N, |a| {
        assert_eq!(fixpow2div2_d(a), unsafe { cref_fixpow2div2_D(a) });
        assert_eq!(fixpow2_d(a), unsafe { cref_fixpow2_D(a) }, "fixpow2_D {a:#x}");
    });
    each_s(105, N, |a| {
        assert_eq!(fixpow2div2_s(a), unsafe { cref_fixpow2div2_S(a) });
        assert_eq!(fixpow2_s(a), unsafe { cref_fixpow2_S(a) }, "fixpow2_S {a}");
    });
}
