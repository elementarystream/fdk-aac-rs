// Differential tests of fdk_aac_rs_fdk::archdef against the C++ reference.

use crate::common_fix::*;
use fdk_aac_rs_cref::fdk_archdef::*;
use fdk_aac_rs_fdk::archdef::*;
use fdk_aac_rs_fdk::common_fix::FixpSpk;
use fdk_aac_rs_fdk::{Arch, ARCH};

#[test]
fn STC_WTC() {
    each_dd(1001, N, |a, b| {
        assert_eq!(stc(a), unsafe { cref_STC(a) });
        assert_eq!(wtc(a), unsafe { cref_WTC(a) });
        assert_eq!(fx_dbl2fx_wtb(a), unsafe { cref_FX_DBL2FX_WTB(a) });
        let (mut re, mut im) = (0, 0);
        let size = unsafe { cref_STCP(a, b, &mut re, &mut im) };
        assert_eq!(size as usize, core::mem::size_of::<FixpStp>());
        assert_eq!(stcp(a, b), FixpSpk { re, im });
        let size = unsafe { cref_WTCP(a, b, &mut re, &mut im) };
        assert_eq!(size as usize, core::mem::size_of::<FixpWtp>());
        assert_eq!(wtcp(a, b), FixpSpk { re, im });
    });
    assert_eq!(unsafe { cref_sizeof_FIXP_STB() } as usize, core::mem::size_of::<FixpStb>());
    assert_eq!(unsafe { cref_sizeof_FIXP_WTB() } as usize, core::mem::size_of::<FixpWtb>());
}

/// The table configuration is the same on every variant; the architecture macros match the
/// variant of the port.
#[test]
fn configuration() {
    let f = unsafe { cref_archdef_flags() };
    assert_eq!(f & 31, 31, "ARCH_PREFER_MULT_32x16, SINETABLE_16BIT, POW2COEFF_16BIT, LDCOEFF_16BIT, WINDOWTABLE_16BIT");
    match ARCH {
        Arch::Aarch64 => assert_eq!(f & 96, 32),
        Arch::X86 | Arch::X86_32 => assert_eq!(f & 96, 64),
        Arch::Generic => assert_eq!(f & 96, 0),
    }
}
