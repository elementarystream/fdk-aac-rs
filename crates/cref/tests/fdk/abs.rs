// Differential tests of fdk_aac_rs_fdk::abs against the C++ reference.

use crate::common_fix::*;
use fdk_aac_rs_cref::fdk_abs::*;
use fdk_aac_rs_fdk::abs::*;
use fdk_aac_rs_fdk::{Arch, ARCH};

#[test]
fn fixabs_D_I() {
    each_d(301, N, |x| {
        assert_eq!(fixabs_d(x), unsafe { cref_fixabs_D(x) }, "fixabs_D({x:#x})");
        assert_eq!(fixabs_i(x), unsafe { cref_fixabs_I(x) }, "fixabs_I({x:#x})");
    });
    // The negation of INT_MIN wraps.
    assert_eq!(unsafe { cref_fixabs_D(i32::MIN) }, i32::MIN);
}

#[test]
fn fixabs_S() {
    each_s(302, N, |x| {
        let r = unsafe { cref_fixabs_S(x) };
        // x86/abs_x86.h returns the INT 32768 for -32768; the generic FIXP_SGL version and every
        // caller (fAbs) truncate it to FIXP_SGL.
        match ARCH {
            // abs_x86.h is used on x86-64 only; 32-bit x86 compiles the generic abs.h.
            Arch::X86 => assert_eq!(r, (x as i32).abs(), "fixabs_S({x})"),
            Arch::Aarch64 | Arch::X86_32 | Arch::Generic => {
                assert_eq!(r, fixabs_s(x) as i32, "fixabs_S({x})")
            }
        }
        assert_eq!(fixabs_s(x), r as i16, "fixabs_S({x})");
    });
}
