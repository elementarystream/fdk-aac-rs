// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libFDK/include/cplx_mul.h, libFDK/include/arm/cplx_mul_arm.h.

//! Port of `cplx_mul.h`: complex multiplications.
//!
//! The C++ functions write the result through two pointers (`cplxMultDiv2(&c_Re, &c_Im, ...)`);
//! the port returns `(c_re, c_im)`. The overloads for `FIXP_SPK` / `FIXP_DPK` twiddles call the
//! overload with the separate components on every variant, so callers pass `w.re, w.im`.
//!
//! `aarch64` (`arm/cplx_mul_arm.h`) replaces the three overloads of `cplxMultDiv2` with a
//! `FIXP_DBL` input `a` (32x32, 32x16 with `FIXP_SGL` coefficients, 32x16 with a `FIXP_SPK`
//! twiddle) by inline assembly: both products of a component are summed in 64 bits
//! (`smull`, `smsubl`/`smaddl`) and shifted once (`asr #32`). The `FIXP_SGL` coefficients are
//! shifted to `FIXP_DBL` first. `x86` and `generic` subtract or add the individually truncated
//! products `fMultDiv2()` in 32 bits. `cplxMultSubDiv2`, `cplxMult` and the 16x16 overloads
//! are the generic code on every variant (but `cplxMult` with `FIXP_DBL` coefficients uses
//! `fMult(FIXP_DBL, FIXP_DBL)`, which differs between the variants, see [`crate::fixmul`]).

use crate::common_fix::{fx_dbl2fx_sgl, FixpDbl, FixpSgl, ARCH_ARM};
use crate::fixmul::*;

/// The `aarch64` assembly of `cplxMultDiv2` (32x32): `smull`, `smsubl` / `smaddl` (the 64 bit
/// sums wrap), `asr #32`, and the low 32 bits of the result.
#[inline(always)]
const fn cplx_mult_div2_arm(a_re: i32, a_im: i32, b_re: i32, b_im: i32) -> (i32, i32) {
    let re = (a_re as i64 * b_re as i64).wrapping_sub(a_im as i64 * b_im as i64);
    let im = (a_re as i64 * b_im as i64).wrapping_add(a_im as i64 * b_re as i64);
    ((re >> 32) as i32, (im >> 32) as i32)
}

/// Overloads of `cplxMultDiv2` that return `FIXP_DBL` values, selected by the types of the input
/// `a` (`Self`) and of the coefficient `b` (`B`), like the C++ overload resolution.
pub trait CplxMultDiv2<B: Copy>: Copy {
    fn cplx_mult_div2(a_re: Self, a_im: Self, b_re: B, b_im: B) -> (FixpDbl, FixpDbl);
}

impl CplxMultDiv2<FixpDbl> for FixpDbl {
    /// `cplxMultDiv2(FIXP_DBL*, FIXP_DBL*, FIXP_DBL, FIXP_DBL, FIXP_DBL, FIXP_DBL)` and the
    /// `FIXP_DPK` overload.
    #[inline(always)]
    fn cplx_mult_div2(a_re: i32, a_im: i32, b_re: i32, b_im: i32) -> (i32, i32) {
        if ARCH_ARM {
            cplx_mult_div2_arm(a_re, a_im, b_re, b_im)
        } else {
            (
                fixmuldiv2_dd(a_re, b_re).wrapping_sub(fixmuldiv2_dd(a_im, b_im)),
                fixmuldiv2_dd(a_re, b_im).wrapping_add(fixmuldiv2_dd(a_im, b_re)),
            )
        }
    }
}

impl CplxMultDiv2<FixpSgl> for FixpDbl {
    /// `cplxMultDiv2(FIXP_DBL*, FIXP_DBL*, FIXP_DBL, FIXP_DBL, FIXP_SGL, FIXP_SGL)` and the
    /// `FIXP_SPK` overload.
    #[inline(always)]
    fn cplx_mult_div2(a_re: i32, a_im: i32, b_re: i16, b_im: i16) -> (i32, i32) {
        if ARCH_ARM {
            cplx_mult_div2_arm(a_re, a_im, (b_re as i32) << 16, (b_im as i32) << 16)
        } else {
            (
                fixmuldiv2_ds(a_re, b_re).wrapping_sub(fixmuldiv2_ds(a_im, b_im)),
                fixmuldiv2_ds(a_re, b_im).wrapping_add(fixmuldiv2_ds(a_im, b_re)),
            )
        }
    }
}

impl CplxMultDiv2<FixpSgl> for FixpSgl {
    /// `cplxMultDiv2(FIXP_DBL*, FIXP_DBL*, FIXP_SGL, FIXP_SGL, FIXP_SGL, FIXP_SGL)` and the
    /// `FIXP_SPK` overload (generic on every variant).
    #[inline(always)]
    fn cplx_mult_div2(a_re: i16, a_im: i16, b_re: i16, b_im: i16) -> (i32, i32) {
        (
            fixmuldiv2_ss(a_re, b_re).wrapping_sub(fixmuldiv2_ss(a_im, b_im)),
            fixmuldiv2_ss(a_re, b_im).wrapping_add(fixmuldiv2_ss(a_im, b_re)),
        )
    }
}

/// `cplxMultDiv2(&c_Re, &c_Im, a_Re, a_Im, b_Re, b_Im)` with `FIXP_DBL` results: `c = a * b / 2`.
#[inline(always)]
pub fn cplx_mult_div2<A: CplxMultDiv2<B>, B: Copy>(
    a_re: A,
    a_im: A,
    b_re: B,
    b_im: B,
) -> (FixpDbl, FixpDbl) {
    A::cplx_mult_div2(a_re, a_im, b_re, b_im)
}

/// `cplxMultDiv2(FIXP_SGL*, FIXP_SGL*, FIXP_SGL, FIXP_SGL, FIXP_SGL, FIXP_SGL)` and the
/// `FIXP_SPK` overload: `FX_DBL2FX_SGL()` of the 16x16 result.
#[inline(always)]
pub const fn cplx_mult_div2_sgl(a_re: i16, a_im: i16, b_re: i16, b_im: i16) -> (i16, i16) {
    (
        fx_dbl2fx_sgl(fixmuldiv2_ss(a_re, b_re).wrapping_sub(fixmuldiv2_ss(a_im, b_im))),
        fx_dbl2fx_sgl(fixmuldiv2_ss(a_re, b_im).wrapping_add(fixmuldiv2_ss(a_im, b_re))),
    )
}

/// Overloads of `cplxMultSubDiv2` and `cplxMult` with a `FIXP_DBL` input `a`, selected by the type
/// of the coefficient `b` (`Self`).
pub trait CplxMult: Copy {
    /// `fMultDiv2(FIXP_DBL, Self)`
    fn mul_div2(a: FixpDbl, b: Self) -> FixpDbl;
    /// `fMult(FIXP_DBL, Self)`
    fn mul(a: FixpDbl, b: Self) -> FixpDbl;
}

impl CplxMult for FixpDbl {
    #[inline(always)]
    fn mul_div2(a: i32, b: i32) -> i32 {
        fixmuldiv2_dd(a, b)
    }
    #[inline(always)]
    fn mul(a: i32, b: i32) -> i32 {
        fixmul_dd(a, b)
    }
}

impl CplxMult for FixpSgl {
    #[inline(always)]
    fn mul_div2(a: i32, b: i16) -> i32 {
        fixmuldiv2_ds(a, b)
    }
    #[inline(always)]
    fn mul(a: i32, b: i16) -> i32 {
        fixmul_ds(a, b)
    }
}

/// `cplxMultSubDiv2(&c_Re, &c_Im, a_Re, a_Im, b_Re, b_Im)` (32x16 and 32x32, generic on every
/// variant): returns `(c_re - re, c_im - im)` where `(re, im)` are the individually truncated
/// products of `a * b / 2`.
#[inline(always)]
pub fn cplx_mult_sub_div2<B: CplxMult>(
    c_re: FixpDbl,
    c_im: FixpDbl,
    a_re: FixpDbl,
    a_im: FixpDbl,
    b_re: B,
    b_im: B,
) -> (FixpDbl, FixpDbl) {
    let re = B::mul_div2(a_re, b_re).wrapping_sub(B::mul_div2(a_im, b_im));
    let im = B::mul_div2(a_re, b_im).wrapping_add(B::mul_div2(a_im, b_re));
    (c_re.wrapping_sub(re), c_im.wrapping_sub(im))
}

/// `cplxMult(&c_Re, &c_Im, a_Re, a_Im, b_Re, b_Im)` with a `FIXP_DBL` input (32x16, 32x32 and
/// the `FIXP_SPK` / `FIXP_DPK` overloads): `c = a * b` from the individual `fMult()` products.
#[inline(always)]
pub fn cplx_mult<B: CplxMult>(
    a_re: FixpDbl,
    a_im: FixpDbl,
    b_re: B,
    b_im: B,
) -> (FixpDbl, FixpDbl) {
    (
        B::mul(a_re, b_re).wrapping_sub(B::mul(a_im, b_im)),
        B::mul(a_re, b_im).wrapping_add(B::mul(a_im, b_re)),
    )
}

/// `cplxMult(FIXP_SGL*, FIXP_SGL*, FIXP_SGL, FIXP_SGL, FIXP_SGL, FIXP_SGL)`.
#[inline(always)]
pub const fn cplx_mult_sgl(a_re: i16, a_im: i16, b_re: i16, b_im: i16) -> (i16, i16) {
    (
        fx_dbl2fx_sgl(fixmul_ss(a_re, b_re).wrapping_sub(fixmul_ss(a_im, b_im))),
        fx_dbl2fx_sgl(fixmul_ss(a_re, b_im).wrapping_add(fixmul_ss(a_im, b_re))),
    )
}
