// Differential tests of fdk_aac_rs_fdk::cplx_mul against the C++ reference: every overload of
// cplxMultDiv2, cplxMultSubDiv2 and cplxMult.

use crate::common_fix::*;
use fdk_aac_rs_cref::fdk_cplx_mul::*;
use fdk_aac_rs_cref::testutil::Rng;
use fdk_aac_rs_fdk::cplx_mul::*;
use fdk_aac_rs_fdk::{Arch, ARCH};

const E_D: &[i32] = &[0, 1, -1, i32::MAX, i32::MIN, i32::MIN + 1, 0x4000_0000, -0x4000_0000, 0x1234_5678];
const E_S: &[i16] = &[0, 1, -1, i16::MAX, i16::MIN, i16::MIN + 1, 0x4000, -0x4000, 0x5A82];

fn each4<T: Copy>(edges: &[T], seed: u64, mut gen: impl FnMut(&mut Rng) -> T, mut f: impl FnMut([T; 4])) {
    for &a in edges {
        for &b in edges {
            for &c in edges {
                for &d in edges {
                    f([a, b, c, d]);
                }
            }
        }
    }
    let mut r = Rng::new(seed);
    for _ in 0..N {
        f([gen(&mut r), gen(&mut r), gen(&mut r), gen(&mut r)]);
    }
}

fn each_d2_s2(seed: u64, mut f: impl FnMut(i32, i32, i16, i16)) {
    for &a in E_D {
        for &b in E_D {
            for &c in E_S {
                for &d in E_S {
                    f(a, b, c, d);
                }
            }
        }
    }
    let mut r = Rng::new(seed);
    for _ in 0..N {
        f(r.i32(), r.i32(), r.i16(), r.i16());
    }
}

macro_rules! cref2 {
    ($f:ident, $t:ty, $($arg:expr),*) => {{
        let (mut re, mut im): ($t, $t) = (0, 0);
        unsafe { $f(&mut re, &mut im, $($arg),*) };
        (re, im)
    }};
}

#[test]
fn cplxMultDiv2_32x32() {
    each4(E_D, 601, |r| r.i32(), |[ar, ai, br, bi]| {
        let p = cplx_mult_div2(ar, ai, br, bi);
        assert_eq!(p, cref2!(cref_cplxMultDiv2_D_D, i32, ar, ai, br, bi), "{ar} {ai} {br} {bi}");
        assert_eq!(p, cref2!(cref_cplxMultDiv2_D_DPK, i32, ar, ai, br, bi));
    });
}

#[test]
fn cplxMultDiv2_32x16() {
    each_d2_s2(602, |ar, ai, br, bi| {
        let p = cplx_mult_div2(ar, ai, br, bi);
        assert_eq!(p, cref2!(cref_cplxMultDiv2_D_S, i32, ar, ai, br, bi), "{ar} {ai} {br} {bi}");
        assert_eq!(p, cref2!(cref_cplxMultDiv2_D_SPK, i32, ar, ai, br, bi));
    });
}

#[test]
fn cplxMultDiv2_16x16() {
    each4(E_S, 603, |r| r.i16(), |[ar, ai, br, bi]| {
        let p = cplx_mult_div2(ar, ai, br, bi);
        assert_eq!(p, cref2!(cref_cplxMultDiv2_S_S, i32, ar, ai, br, bi), "{ar} {ai} {br} {bi}");
        assert_eq!(p, cref2!(cref_cplxMultDiv2_S_SPK, i32, ar, ai, br, bi));
        let q = cplx_mult_div2_sgl(ar, ai, br, bi);
        assert_eq!(q, cref2!(cref_cplxMultDiv2_S_S_sgl, i16, ar, ai, br, bi));
        assert_eq!(q, cref2!(cref_cplxMultDiv2_S_SPK_sgl, i16, ar, ai, br, bi));
    });
}

#[test]
fn cplxMultSubDiv2() {
    let mut r = Rng::new(604);
    each_d2_s2(605, |ar, ai, br, bi| {
        let (cr, ci) = (r.i32(), r.i32());
        let (mut re, mut im) = (cr, ci);
        unsafe { cref_cplxMultSubDiv2_D_S(&mut re, &mut im, ar, ai, br, bi) };
        assert_eq!(cplx_mult_sub_div2(cr, ci, ar, ai, br, bi), (re, im));
    });
    each4(E_D, 606, |r| r.i32(), |[ar, ai, br, bi]| {
        let (cr, ci) = (r.i32(), r.i32());
        let (mut re, mut im) = (cr, ci);
        unsafe { cref_cplxMultSubDiv2_D_D(&mut re, &mut im, ar, ai, br, bi) };
        assert_eq!(cplx_mult_sub_div2(cr, ci, ar, ai, br, bi), (re, im));
    });
}

#[test]
fn cplxMult() {
    each_d2_s2(607, |ar, ai, br, bi| {
        let p = cplx_mult(ar, ai, br, bi);
        assert_eq!(p, cref2!(cref_cplxMult_D_S, i32, ar, ai, br, bi), "{ar} {ai} {br} {bi}");
        assert_eq!(p, cref2!(cref_cplxMult_D_SPK, i32, ar, ai, br, bi));
    });
    each4(E_D, 608, |r| r.i32(), |[ar, ai, br, bi]| {
        let p = cplx_mult(ar, ai, br, bi);
        assert_eq!(p, cref2!(cref_cplxMult_D_D, i32, ar, ai, br, bi), "{ar} {ai} {br} {bi}");
        assert_eq!(p, cref2!(cref_cplxMult_D_DPK, i32, ar, ai, br, bi));
    });
    each4(E_S, 609, |r| r.i16(), |[ar, ai, br, bi]| {
        let p = cplx_mult_sgl(ar, ai, br, bi);
        assert_eq!(p, cref2!(cref_cplxMult_S_S_sgl, i16, ar, ai, br, bi), "{ar} {ai} {br} {bi}");
    });
}

/// The variant difference: the AArch64 assembly sums the 64 bit products before the shift.
#[test]
fn cplxMultDiv2_variant() {
    // re = (1 * 1 - 1 * (-1)) >> 32: 0 on aarch64; fMultDiv2(1, 1) - fMultDiv2(1, -1) = 0 - (-1)
    // = 1 on x86.
    let p = cref2!(cref_cplxMultDiv2_D_D, i32, 1, 1, 1, -1);
    match ARCH {
        Arch::Aarch64 => assert_eq!(p.0, 0),
        Arch::X86 | Arch::X86_32 | Arch::Generic => assert_eq!(p.0, 1),
    }
    assert_eq!(cplx_mult_div2(1, 1, 1, -1), p);
}
