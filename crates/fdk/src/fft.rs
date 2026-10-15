// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libFDK/include/fft.h, libFDK/src/fft.cpp.

//! Port of `fft.h`: in-place complex FFTs of the lengths the library uses.
//!
//! The data are interleaved real/imaginary `FIXP_DBL` values. [`fft`] dispatches on the length
//! and adds the scaling of the kernel to the exponent: the power-of-two lengths 64 to 512 use the
//! radix-2 [`dit_fft`], 16 and 32 the unrolled kernels [`fft_16`] and [`fft_32`], the other
//! lengths small Winograd kernels ([`fft3`], [`fft5`], ...) or two-dimensional decompositions
//! (`fftN2` with rotation vectors).
//!
//! # Variants
//!
//! `fft.cpp:719-725` selects where the halving of the "Re C + Re D" sums of [`fft_16`] and
//! [`fft_32`] happens (`SHIFT_A`, `SHIFT_B`): on `__arm__` (`aarch64`) after the addition,
//! `(a + b) >> 1`, elsewhere (`x86`, `generic`) before it, `(a >> 1) + (b >> 1)`. In [`fft_16`]
//! the two forms differ in the least significant bit and in overflow; in [`fft_32`] the sums are
//! written `(a + b) SHIFT_A`, so both variants compute `(a + b) >> 1` (at a different place).
//! The other differences between the variants come from the primitives (`fMult`, `cplxMultDiv2`)
//! and from [`dit_fft`]. No `FUNCTION_*` override is defined in either build: the `.ii` files
//! compile the C functions of `fft.h`/`fft.cpp` (the fft12 based on `fftN2` is replaced by the
//! unrolled one, as `FUNCTION_fft12` is defined by it).
//!
//! # Overflow
//!
//! The kernels expect one or two bits of headroom; with full-scale input the additions overflow
//! (undefined behavior in C). The port computes in [`Wrapping<i32>`] (the slices are
//! reinterpreted, see `as_wrapping`), with arithmetic shifts; the `cref` tests with full-scale
//! input (constant and alternating `MINVAL`/`MAXVAL`, impulses) give the same values as both
//! reference builds for every length.

use core::num::Wrapping;
use core::ops::{Add, AddAssign, Neg, Shl, Shr, Sub, SubAssign};

use crate::archdef::{stc, FixpStb, FixpStp};
use crate::common_fix::*;
use crate::fft_rad2::dit_fft;
use crate::tools_rom::*;

/// A `FIXP_DBL` with the wrap-around arithmetic of the reference builds.
type W = Wrapping<i32>;

/// Reinterprets `FIXP_DBL` values as [`W`].
#[inline(always)]
pub(crate) fn as_wrapping(x: &mut [FixpDbl]) -> &mut [W] {
    // SAFETY: `Wrapping<i32>` is `repr(transparent)` over `i32`, so the slices have the same
    // layout; the lifetime and the exclusive borrow are carried over.
    unsafe { &mut *(x as *mut [FixpDbl] as *mut [W]) }
}

/// The value type of the kernels: one `FIXP_DBL` ([`W`], the straight port) or several
/// independent transforms in the lanes of a vector (`neon::V4`), with the wrap-around
/// arithmetic of the reference builds and the primitives of the selected variant.
///
/// The kernels are generic over it; their code is the straight port, and every operation acts
/// on each lane exactly as on a scalar `FIXP_DBL`.
pub(crate) trait Lane:
    Copy
    + Add<Output = Self>
    + Sub<Output = Self>
    + Neg<Output = Self>
    + Shr<usize, Output = Self>
    + Shl<usize, Output = Self>
    + AddAssign
    + SubAssign
{
    /// 0
    const ZERO: Self;
    /// `fMultDiv2(FIXP_DBL, FIXP_SGL)`
    fn fmd2(self, b: FixpSgl) -> Self;
    /// `fMult(FIXP_DBL, FIXP_SGL)`
    fn fm(self, b: FixpSgl) -> Self;
    /// `cplxMultDiv2(&c_re, &c_im, a_re, a_im, b_re, b_im)` with `FIXP_SGL` coefficients.
    fn cmd2_s(a_re: Self, a_im: Self, b_re: FixpSgl, b_im: FixpSgl) -> (Self, Self);
}

impl Lane for W {
    const ZERO: W = Wrapping(0);
    #[inline(always)]
    fn fmd2(self, b: FixpSgl) -> W {
        Wrapping(f_mult_div2(self.0, b))
    }
    #[inline(always)]
    fn fm(self, b: FixpSgl) -> W {
        Wrapping(f_mult(self.0, b))
    }
    #[inline(always)]
    fn cmd2_s(a_re: W, a_im: W, b_re: FixpSgl, b_im: FixpSgl) -> (W, W) {
        let (re, im) = cplx_mult_div2(a_re.0, a_im.0, b_re, b_im);
        (Wrapping(re), Wrapping(im))
    }
}

const ARM: bool = ARCH_ARM;

/// `(x SHIFT_A)`: empty on `__arm__`, `>> 1` elsewhere.
#[inline(always)]
fn shift_a<D: Lane>(x: D) -> D {
    if ARM {
        x
    } else {
        x >> 1
    }
}

/// `(x SHIFT_B)`: `>> 1` on `__arm__`, empty elsewhere.
#[inline(always)]
fn shift_b<D: Lane>(x: D) -> D {
    if ARM {
        x >> 1
    } else {
        x
    }
}

/// `fMultDiv2(FIXP_DBL, FIXP_SGL)`
#[inline(always)]
fn fmd2<D: Lane>(a: D, b: FixpSgl) -> D {
    a.fmd2(b)
}

/// `fMult(FIXP_DBL, FIXP_SGL)`
#[inline(always)]
fn fm<D: Lane>(a: D, b: FixpSgl) -> D {
    a.fm(b)
}

/// `cplxMultDiv2(&c_re, &c_im, a_re, a_im, w)` with a `FIXP_SPK` twiddle: returns `(c_re, c_im)`.
#[inline(always)]
fn cmd2<D: Lane>(a_re: D, a_im: D, w: FixpStp) -> (D, D) {
    D::cmd2_s(a_re, a_im, w.re, w.im)
}

/// `cplxMultDiv2(&c_re, &c_im, a_re, a_im, b_re, b_im)` with `FIXP_SGL` coefficients.
#[inline(always)]
fn cmd2_s<D: Lane>(a_re: D, a_im: D, b_re: FixpSgl, b_im: FixpSgl) -> (D, D) {
    D::cmd2_s(a_re, a_im, b_re, b_im)
}

/// `W_PiFOURTH`: `STC(0x5a82799a)`
const W_PI_FOURTH: FixpSgl = stc(0x5a82799a);

/// `SUMDIFF_PIFOURTH(diff, sum, a, b)`: returns `(diff, sum)`.
#[inline(always)]
fn sumdiff_pifourth<D: Lane>(a: D, b: D) -> (D, D) {
    let wa = fmd2(a, W_PI_FOURTH);
    let wb = fmd2(b, W_PI_FOURTH);
    (wb - wa, wb + wa)
}

pub const SCALEFACTOR2048: i32 = 10;
pub const SCALEFACTOR1024: i32 = 9;
pub const SCALEFACTOR512: i32 = 8;
pub const SCALEFACTOR256: i32 = 7;
pub const SCALEFACTOR128: i32 = 6;
pub const SCALEFACTOR64: i32 = 5;
pub const SCALEFACTOR32: i32 = 4;
pub const SCALEFACTOR16: i32 = 3;
pub const SCALEFACTOR8: i32 = 2;
pub const SCALEFACTOR4: i32 = 1;
pub const SCALEFACTOR2: i32 = 1;

pub const SCALEFACTOR3: i32 = 1;
pub const SCALEFACTOR5: i32 = 1;
pub const SCALEFACTOR6: i32 = SCALEFACTOR2 + SCALEFACTOR3 + 2;
pub const SCALEFACTOR7: i32 = 2;
pub const SCALEFACTOR9: i32 = 2;
pub const SCALEFACTOR10: i32 = 5;
pub const SCALEFACTOR12: i32 = 3;
pub const SCALEFACTOR15: i32 = 3;
pub const SCALEFACTOR18: i32 = SCALEFACTOR2 + SCALEFACTOR9 + 2;
pub const SCALEFACTOR20: i32 = SCALEFACTOR4 + SCALEFACTOR5 + 2;
pub const SCALEFACTOR21: i32 = SCALEFACTOR3 + SCALEFACTOR7 + 2;
pub const SCALEFACTOR24: i32 = SCALEFACTOR2 + SCALEFACTOR12 + 2;
pub const SCALEFACTOR30: i32 = SCALEFACTOR2 + SCALEFACTOR15 + 2;
pub const SCALEFACTOR40: i32 = SCALEFACTOR5 + SCALEFACTOR8 + 2;
pub const SCALEFACTOR48: i32 = SCALEFACTOR4 + SCALEFACTOR12 + 2;
pub const SCALEFACTOR60: i32 = SCALEFACTOR4 + SCALEFACTOR15 + 2;
pub const SCALEFACTOR80: i32 = SCALEFACTOR5 + SCALEFACTOR16 + 2;
pub const SCALEFACTOR96: i32 = SCALEFACTOR3 + SCALEFACTOR32 + 2;
pub const SCALEFACTOR120: i32 = SCALEFACTOR8 + SCALEFACTOR15 + 2;
pub const SCALEFACTOR160: i32 = SCALEFACTOR10 + SCALEFACTOR16 + 2;
pub const SCALEFACTOR168: i32 = SCALEFACTOR21 + SCALEFACTOR8 + 2;
pub const SCALEFACTOR192: i32 = SCALEFACTOR12 + SCALEFACTOR16 + 2;
pub const SCALEFACTOR240: i32 = SCALEFACTOR16 + SCALEFACTOR15 + 2;
pub const SCALEFACTOR320: i32 = SCALEFACTOR10 + SCALEFACTOR32 + 2;
pub const SCALEFACTOR336: i32 = SCALEFACTOR21 + SCALEFACTOR16 + 2;
pub const SCALEFACTOR384: i32 = SCALEFACTOR12 + SCALEFACTOR32 + 2;
pub const SCALEFACTOR480: i32 = SCALEFACTOR32 + SCALEFACTOR15 + 2;

// ---------------------------------------------------------------------------------------------
// fft.h
// ---------------------------------------------------------------------------------------------

/// `fft_4(x)` (`fft.h`): in-place complex FFT of length 4. The input needs one bit of headroom.
pub fn fft_4(x: &mut [FixpDbl]) {
    fft_4_w(as_wrapping(x));
}

#[inline(always)]
pub(crate) fn fft_4_w<D: Lane>(x: &mut [D]) {
    let x = &mut x[..8];
    let a00 = (x[0] + x[4]) >> 1; // Re A + Re B
    let mut a10 = (x[2] + x[6]) >> 1; // Re C + Re D
    let a20 = (x[1] + x[5]) >> 1; // Im A + Im B
    let mut a30 = (x[3] + x[7]) >> 1; // Im C + Im D

    x[0] = a00 + a10; // Re A' = Re A + Re B + Re C + Re D
    x[1] = a20 + a30; // Im A' = Im A + Im B + Im C + Im D

    let tmp0 = a00 - x[4]; // Re A - Re B
    let tmp1 = a20 - x[5]; // Im A - Im B

    x[4] = a00 - a10; // Re C' = Re A + Re B - Re C - Re D
    x[5] = a20 - a30; // Im C' = Im A + Im B - Im C - Im D

    a10 -= x[6]; // Re C - Re D
    a30 -= x[7]; // Im C - Im D

    x[2] = tmp0 + a30; // Re B' = Re A - Re B + Im C - Im D
    x[6] = tmp0 - a30; // Re D' = Re A - Re B - Im C + Im D
    x[3] = tmp1 - a10; // Im B' = Im A - Im B - Re C + Re D
    x[7] = tmp1 + a10; // Im D' = Im A - Im B + Re C - Re D
}

/// `fft_8(x)` (`fft.h`): in-place complex FFT of length 8.
pub fn fft_8(x: &mut [FixpDbl]) {
    fft_8_w(as_wrapping(x));
}

pub(crate) fn fft_8_w<D: Lane>(x: &mut [D]) {
    let x = &mut x[..16];
    // FIXP_SPK w_PiFOURTH = {{FIXP_SGL(0x5A82), FIXP_SGL(0x5A82)}};
    let w_pi_fourth = FixpStp { re: 0x5A82, im: 0x5A82 };

    let mut y = [D::ZERO; 16];

    let mut a00 = (x[0] + x[8]) >> 1;
    let mut a10 = x[4] + x[12];
    let mut a20 = (x[1] + x[9]) >> 1;
    let mut a30 = x[5] + x[13];

    y[0] = a00 + (a10 >> 1);
    y[4] = a00 - (a10 >> 1);
    y[1] = a20 + (a30 >> 1);
    y[5] = a20 - (a30 >> 1);

    a00 -= x[8];
    a10 = (a10 >> 1) - x[12];
    a20 -= x[9];
    a30 = (a30 >> 1) - x[13];

    y[2] = a00 + a30;
    y[6] = a00 - a30;
    y[3] = a20 - a10;
    y[7] = a20 + a10;

    a00 = (x[2] + x[10]) >> 1;
    a10 = x[6] + x[14];
    a20 = (x[3] + x[11]) >> 1;
    a30 = x[7] + x[15];

    y[8] = a00 + (a10 >> 1);
    y[12] = a00 - (a10 >> 1);
    y[9] = a20 + (a30 >> 1);
    y[13] = a20 - (a30 >> 1);

    a00 -= x[10];
    a10 = (a10 >> 1) - x[14];
    a20 -= x[11];
    a30 = (a30 >> 1) - x[15];

    y[10] = a00 + a30;
    y[14] = a00 - a30;
    y[11] = a20 - a10;
    y[15] = a20 + a10;

    let mut ur = y[0] >> 1;
    let mut ui = y[1] >> 1;
    let mut vr = y[8];
    let mut vi = y[9];
    x[0] = ur + (vr >> 1);
    x[1] = ui + (vi >> 1);
    x[8] = ur - (vr >> 1);
    x[9] = ui - (vi >> 1);

    ur = y[4] >> 1;
    ui = y[5] >> 1;
    vi = y[12];
    vr = y[13];
    x[4] = ur + (vr >> 1);
    x[5] = ui - (vi >> 1);
    x[12] = ur - (vr >> 1);
    x[13] = ui + (vi >> 1);

    ur = y[10];
    ui = y[11];

    (vi, vr) = cmd2(ui, ur, w_pi_fourth);

    ur = y[2];
    ui = y[3];
    x[2] = (ur >> 1) + vr;
    x[3] = (ui >> 1) + vi;
    x[10] = (ur >> 1) - vr;
    x[11] = (ui >> 1) - vi;

    ur = y[14];
    ui = y[15];

    (vr, vi) = cmd2(ui, ur, w_pi_fourth);

    ur = y[6];
    ui = y[7];
    x[6] = (ur >> 1) + vr;
    x[7] = (ui >> 1) - vi;
    x[14] = (ur >> 1) - vr;
    x[15] = (ui >> 1) + vi;
}

// ---------------------------------------------------------------------------------------------
// fft.cpp: small kernels
// ---------------------------------------------------------------------------------------------

/// `fft2(pDat)`: FFT of length 2. Input vector unscaled, output vector scaled with factor 0.5.
pub fn fft2(p_dat: &mut [FixpDbl]) {
    fft2_w(as_wrapping(p_dat));
}

pub(crate) fn fft2_w<D: Lane>(p_dat: &mut [D]) {
    let p_dat = &mut p_dat[..4];
    // real part
    let r1 = p_dat[2];
    let r2 = p_dat[0];

    // imaginary part
    let i1 = p_dat[3];
    let i2 = p_dat[1];

    // real part
    p_dat[0] = (r2 + r1) >> 1;
    p_dat[2] = (r2 - r1) >> 1;

    // imaginary part
    p_dat[1] = (i2 + i1) >> 1;
    p_dat[3] = (i2 - i1) >> 1;
}

/// `C31`: `STC(0x91261468)` = `FL2FXCONST_DBL(-0.86602540)` = -sqrt(3)/2
const C31: FixpSgl = stc(0x91261468u32 as i32);

/// `fft3(pDat)`: FFT of length 3 according to the algorithm after Winograd.
pub fn fft3(p_dat: &mut [FixpDbl]) {
    fft3_w(as_wrapping(p_dat));
}

pub(crate) fn fft3_w<D: Lane>(p_dat: &mut [D]) {
    let p_dat = &mut p_dat[..6];
    // real part
    let mut r1 = p_dat[2] + p_dat[4];
    let r2 = fmd2(p_dat[2] - p_dat[4], C31);
    let mut p_d = p_dat[0] >> 1;
    p_dat[0] = p_d + (r1 >> 1);
    r1 = p_d - (r1 >> 2);

    // imaginary part
    let mut s1 = p_dat[3] + p_dat[5];
    let s2 = fmd2(p_dat[3] - p_dat[5], C31);
    p_d = p_dat[1] >> 1;
    p_dat[1] = p_d + (s1 >> 1);
    s1 = p_d - (s1 >> 2);

    // combination
    p_dat[2] = r1 - s2;
    p_dat[4] = r1 + s2;
    p_dat[3] = s1 + r2;
    p_dat[5] = s1 - r2;
}

/// `C51`: `F5C(0x79bc3854)` = `FL2FXCONST_DBL( 0.95105652)`
const C51: FixpSgl = stc(0x79bc3854);
/// `C52`: `F5C(0x9d839db0)` = `FL2FXCONST_DBL(-1.53884180/2)`
const C52: FixpSgl = stc(0x9d839db0u32 as i32);
/// `C53`: `F5C(0xd18053ce)` = `FL2FXCONST_DBL(-0.36327126)`
const C53: FixpSgl = stc(0xd18053ceu32 as i32);
/// `C54`: `F5C(0x478dde64)` = `FL2FXCONST_DBL( 0.55901699)`
const C54: FixpSgl = stc(0x478dde64);
/// `C55`: `F5C(0xb0000001)` = `FL2FXCONST_DBL(-1.25/2)`
const C55: FixpSgl = stc(0xb0000001u32 as i32);

/// `fft5(pDat)`: FFT of length 5 according to the algorithm after Winograd (with a prescale of 2
/// instead of 3).
pub fn fft5(p_dat: &mut [FixpDbl]) {
    fft5_w(as_wrapping(p_dat));
}

pub(crate) fn fft5_w<D: Lane>(p_dat: &mut [D]) {
    let p_dat = &mut p_dat[..10];
    // real part
    let mut r1 = (p_dat[2] + p_dat[8]) >> 1;
    let mut r4 = (p_dat[2] - p_dat[8]) >> 1;
    let mut r3 = (p_dat[4] + p_dat[6]) >> 1;
    let mut r2 = (p_dat[4] - p_dat[6]) >> 1;
    let mut t = fm(r1 - r3, C54);
    r1 += r3;
    p_dat[0] = (p_dat[0] >> 1) + r1;
    // Bit shift left because of the constant C55 which was scaled with the factor 0.5 because
    // of the representation of the values as fracts
    r1 = p_dat[0] + (fmd2(r1, C55) << 2);
    r3 = r1 - t;
    r1 += t;
    t = fm(r4 + r2, C51);
    // Bit shift left because of the constant C55 which was scaled with the factor 0.5 because
    // of the representation of the values as fracts
    r4 = t + (fmd2(r4, C52) << 2);
    r2 = t + fm(r2, C53);

    // imaginary part
    let mut s1 = (p_dat[3] + p_dat[9]) >> 1;
    let mut s4 = (p_dat[3] - p_dat[9]) >> 1;
    let mut s3 = (p_dat[5] + p_dat[7]) >> 1;
    let mut s2 = (p_dat[5] - p_dat[7]) >> 1;
    t = fm(s1 - s3, C54);
    s1 += s3;
    p_dat[1] = (p_dat[1] >> 1) + s1;
    // Bit shift left because of the constant C55 which was scaled with the factor 0.5 because
    // of the representation of the values as fracts
    s1 = p_dat[1] + (fmd2(s1, C55) << 2);
    s3 = s1 - t;
    s1 += t;
    t = fm(s4 + s2, C51);
    // Bit shift left because of the constant C55 which was scaled with the factor 0.5 because
    // of the representation of the values as fracts
    s4 = t + (fmd2(s4, C52) << 2);
    s2 = t + fm(s2, C53);

    // combination
    p_dat[2] = r1 + s2;
    p_dat[8] = r1 - s2;
    p_dat[4] = r3 - s4;
    p_dat[6] = r3 + s4;

    p_dat[3] = s1 - r2;
    p_dat[9] = s1 + r2;
    p_dat[5] = s3 + r4;
    p_dat[7] = s3 - r4;
}

/// `fft10(x)`: complex 10-point FFT, in place. The result is scaled by `SCALEFACTOR10` bits.
pub fn fft10(x: &mut [FixpDbl]) {
    fft10_w(as_wrapping(x));
}

pub(crate) fn fft10_w<D: Lane>(x: &mut [D]) {
    let x = &mut x[..20];
    const S: usize = 1; // stride factor
    const SF: usize = SCALEFACTOR10 as usize;

    // 2 fft5 stages

    // real part
    let mut x0 = x[S * 0] >> SF;
    let mut x1 = x[S * 4] >> SF;
    let mut x2 = x[S * 8] >> SF;
    let mut x3 = x[S * 12] >> SF;
    let mut x4 = x[S * 16] >> SF;

    let mut r1 = x3 + x2;
    let mut r4 = x3 - x2;
    let mut r3 = x1 + x4;
    let mut r2 = x1 - x4;
    let mut t = fm(r1 - r3, C54);
    r1 += r3;
    let y00 = x0 + r1;
    r1 = y00 + (fm(r1, C55) << 1);
    r3 = r1 - t;
    r1 += t;
    t = fm(r4 + r2, C51);
    r4 = t + (fm(r4, C52) << 1);
    r2 = t + fm(r2, C53);

    // imaginary part
    x0 = x[S * 0 + 1] >> SF;
    x1 = x[S * 4 + 1] >> SF;
    x2 = x[S * 8 + 1] >> SF;
    x3 = x[S * 12 + 1] >> SF;
    x4 = x[S * 16 + 1] >> SF;

    let mut s1 = x3 + x2;
    let mut s4 = x3 - x2;
    let mut s3 = x1 + x4;
    let mut s2 = x1 - x4;
    t = fm(s1 - s3, C54);
    s1 += s3;
    let y01 = x0 + s1;
    s1 = y01 + (fm(s1, C55) << 1);
    s3 = s1 - t;
    s1 += t;
    t = fm(s4 + s2, C51);
    s4 = t + (fm(s4, C52) << 1);
    s2 = t + fm(s2, C53);

    // combination
    let y04 = r1 + s2;
    let y16 = r1 - s2;
    let y08 = r3 - s4;
    let y12 = r3 + s4;

    let y05 = s1 - r2;
    let y17 = s1 + r2;
    let y09 = s3 + r4;
    let y13 = s3 - r4;

    // real part
    x0 = x[S * 10] >> SF;
    x1 = x[S * 2] >> SF;
    x2 = x[S * 6] >> SF;
    x3 = x[S * 14] >> SF;
    x4 = x[S * 18] >> SF;

    r1 = x1 + x4;
    r4 = x1 - x4;
    r3 = x3 + x2;
    r2 = x3 - x2;
    t = fm(r1 - r3, C54);
    r1 += r3;
    let y02 = x0 + r1;
    r1 = y02 + (fm(r1, C55) << 1);
    r3 = r1 - t;
    r1 += t;
    t = fm(r4 + r2, C51);
    r4 = t + (fm(r4, C52) << 1);
    r2 = t + fm(r2, C53);

    // imaginary part
    x0 = x[S * 10 + 1] >> SF;
    x1 = x[S * 2 + 1] >> SF;
    x2 = x[S * 6 + 1] >> SF;
    x3 = x[S * 14 + 1] >> SF;
    x4 = x[S * 18 + 1] >> SF;

    s1 = x1 + x4;
    s4 = x1 - x4;
    s3 = x3 + x2;
    s2 = x3 - x2;
    t = fm(s1 - s3, C54);
    s1 += s3;
    let y03 = x0 + s1;
    s1 = y03 + (fm(s1, C55) << 1);
    s3 = s1 - t;
    s1 += t;
    t = fm(s4 + s2, C51);
    s4 = t + (fm(s4, C52) << 1);
    s2 = t + fm(s2, C53);

    // combination
    let y06 = r1 + s2;
    let y18 = r1 - s2;
    let y10 = r3 - s4;
    let y14 = r3 + s4;

    let y07 = s1 - r2;
    let y19 = s1 + r2;
    let y11 = s3 + r4;
    let y15 = s3 - r4;

    // 5 fft2 stages
    x[S * 0] = y00 + y02;
    x[S * 0 + 1] = y01 + y03;
    x[S * 10] = y00 - y02;
    x[S * 10 + 1] = y01 - y03;

    x[S * 4] = y04 + y06;
    x[S * 4 + 1] = y05 + y07;
    x[S * 14] = y04 - y06;
    x[S * 14 + 1] = y05 - y07;

    x[S * 8] = y08 + y10;
    x[S * 8 + 1] = y09 + y11;
    x[S * 18] = y08 - y10;
    x[S * 18 + 1] = y09 - y11;

    x[S * 12] = y12 + y14;
    x[S * 12 + 1] = y13 + y15;
    x[S * 2] = y12 - y14;
    x[S * 2 + 1] = y13 - y15;

    x[S * 16] = y16 + y18;
    x[S * 16 + 1] = y17 + y19;
    x[S * 6] = y16 - y18;
    x[S * 6 + 1] = y17 - y19;
}

/// `fft12(pInput)`: the unrolled FFT of length 12 (`FUNCTION_fft12`): 4 FFTs of length 3, the
/// rotation, 3 FFTs of length 4.
pub fn fft12(p_input: &mut [FixpDbl]) {
    fft12_w(as_wrapping(p_input));
}

pub(crate) fn fft12_w<D: Lane>(p_input: &mut [D]) {
    let p_input = &mut p_input[..24];
    let mut a_dst = [D::ZERO; 24];

    let mut p_src = 0usize;
    let mut p_dst = 0usize;

    // First 3*2 samples are shifted right by 2 before output
    {
        let src = &p_input[p_src..];
        let dst = &mut a_dst[p_dst..];
        let mut r1 = src[8] + src[16];
        let r2 = fmd2(src[8] - src[16], C31);
        let mut p_d = src[0] >> 1;
        dst[0] = (p_d + (r1 >> 1)) >> 1;
        r1 = p_d - (r1 >> 2);

        // imaginary part
        let mut s1 = src[9] + src[17];
        let s2 = fmd2(src[9] - src[17], C31);
        p_d = src[1] >> 1;
        dst[1] = (p_d + (s1 >> 1)) >> 1;
        s1 = p_d - (s1 >> 2);

        // combination
        dst[2] = (r1 - s2) >> 1;
        dst[3] = (s1 + r2) >> 1;
        dst[4] = (r1 + s2) >> 1;
        dst[5] = (s1 - r2) >> 1;
    }
    p_src += 2;
    p_dst += 6;

    let mut p_vec = 0usize;
    for _ in 0..2 {
        // sample 0,1 are shifted right by 2 before output
        // sample 2,3 4,5 are shifted right by 1 and complex multiplied before output
        let src = &p_input[p_src..];
        let dst = &mut a_dst[p_dst..];

        let mut r1 = src[8] + src[16];
        let r2 = fmd2(src[8] - src[16], C31);
        let mut p_d = src[0] >> 1;
        dst[0] = (p_d + (r1 >> 1)) >> 1;
        r1 = p_d - (r1 >> 2);

        // imaginary part
        let mut s1 = src[9] + src[17];
        let s2 = fmd2(src[9] - src[17], C31);
        p_d = src[1] >> 1;
        dst[1] = (p_d + (s1 >> 1)) >> 1;
        s1 = p_d - (s1 >> 2);

        // combination
        let mut re = r1 - s2;
        let mut im = s1 + r2;
        let mut vre: FixpStb = ROT_VECTOR_REAL_12[p_vec];
        let mut vim: FixpStb = ROT_VECTOR_IMAG_12[p_vec];
        p_vec += 1;
        (dst[3], dst[2]) = cmd2_s(im, re, vre, vim);

        re = r1 + s2;
        im = s1 - r2;
        vre = ROT_VECTOR_REAL_12[p_vec];
        vim = ROT_VECTOR_IMAG_12[p_vec];
        p_vec += 1;
        (dst[5], dst[4]) = cmd2_s(im, re, vre, vim);

        p_dst += 6;
        p_src += 2;
    }
    // sample 0,1 are shifted right by 2 before output
    // sample 2,3 is shifted right by 1 and complex multiplied with (0.0,+1.0)
    // sample 4,5 is shifted right by 1 and complex multiplied with (-1.0,0.0)
    {
        let src = &p_input[p_src..];
        let dst = &mut a_dst[p_dst..];
        let mut r1 = src[8] + src[16];
        let r2 = fmd2(src[8] - src[16], C31);
        let mut p_d = src[0] >> 1;
        dst[0] = (p_d + (r1 >> 1)) >> 1;
        r1 = p_d - (r1 >> 2);

        // imaginary part
        let mut s1 = src[9] + src[17];
        let s2 = fmd2(src[9] - src[17], C31);
        p_d = src[1] >> 1;
        dst[1] = (p_d + (s1 >> 1)) >> 1;
        s1 = p_d - (s1 >> 2);

        // combination
        dst[2] = (s1 + r2) >> 1;
        dst[3] = (s2 - r1) >> 1;
        dst[4] = -((r1 + s2) >> 1);
        dst[5] = (r2 - s1) >> 1;
    }

    // Perform 3 times the fft of length 4. The input samples are at the address of aDst and the
    // output samples are at the address of pInput. The input vector for the fft of length 4 is
    // built of the interleaved samples in aDst, the output samples are stored consecutively at
    // the address of pInput.
    let mut p_src = 0usize;
    let mut p_dst = 0usize;
    for _ in 0..3 {
        // inline FFT4 merged with incoming resorting loop
        let src = &a_dst[p_src..];
        let dst = &mut p_input[p_dst..];

        let a00 = (src[0] + src[12]) >> 1; // Re A + Re B
        let mut a10 = (src[6] + src[18]) >> 1; // Re C + Re D
        let a20 = (src[1] + src[13]) >> 1; // Im A + Im B
        let mut a30 = (src[7] + src[19]) >> 1; // Im C + Im D

        dst[0] = a00 + a10; // Re A' = Re A + Re B + Re C + Re D
        dst[1] = a20 + a30; // Im A' = Im A + Im B + Im C + Im D

        let tmp0 = a00 - src[12]; // Re A - Re B
        let tmp1 = a20 - src[13]; // Im A - Im B

        dst[12] = a00 - a10; // Re C' = Re A + Re B - Re C - Re D
        dst[13] = a20 - a30; // Im C' = Im A + Im B - Im C - Im D

        a10 -= src[18]; // Re C - Re D
        a30 -= src[19]; // Im C - Im D

        dst[6] = tmp0 + a30; // Re B' = Re A - Re B + Im C - Im D
        dst[18] = tmp0 - a30; // Re D' = Re A - Re B - Im C + Im D
        dst[7] = tmp1 - a10; // Im B' = Im A - Im B - Re C + Re D
        dst[19] = tmp1 + a10; // Im D' = Im A - Im B + Re C - Re D

        p_src += 2;
        p_dst += 2;
    }
}

const N3: usize = 3;
const N5: usize = 5;
const N6: usize = 6;
const N15: usize = 15;

/// `fft15(pInput)`: FFT of length 15, split into FFTs of length 3 and length 5.
pub fn fft15(p_input: &mut [FixpDbl]) {
    fft15_w(as_wrapping(p_input));
}

pub(crate) fn fft15_w<D: Lane>(p_input: &mut [D]) {
    let p_input = &mut p_input[..2 * N15];
    let mut a_dst = [D::ZERO; 2 * N15];
    let mut a_dst1 = [D::ZERO; 2 * N15];

    // Sort input vector for fft's of length 3
    // input3(0:2)   = [input(0) input(5) input(10)];
    // input3(3:5)   = [input(3) input(8) input(13)];
    // input3(6:8)   = [input(6) input(11) input(1)];
    // input3(9:11)  = [input(9) input(14) input(4)];
    // input3(12:14) = [input(12) input(2) input(7)];
    {
        let p_src = &*p_input;
        let p_dst = &mut a_dst;
        // Merge 3 loops into one, skip call of fft3
        let mut l = 0usize;
        let mut k = 0usize;
        for _ in 0..N5 {
            p_dst[k] = p_src[l];
            p_dst[k + 1] = p_src[l + 1];
            l += 2 * N5;
            if l >= 2 * N15 {
                l -= 2 * N15;
            }

            p_dst[k + 2] = p_src[l];
            p_dst[k + 3] = p_src[l + 1];
            l += 2 * N5;
            if l >= 2 * N15 {
                l -= 2 * N15;
            }
            p_dst[k + 4] = p_src[l];
            p_dst[k + 5] = p_src[l + 1];
            l += (2 * N5) + (2 * N3);
            if l >= 2 * N15 {
                l -= 2 * N15;
            }

            // fft3 merged with shift right by 2 loop

            // real part
            let mut r1 = p_dst[k + 2] + p_dst[k + 4];
            let r2 = fm(p_dst[k + 2] - p_dst[k + 4], C31);
            let mut s1 = p_dst[k];
            p_dst[k] = (s1 + r1) >> 2;
            r1 = s1 - (r1 >> 1);

            // imaginary part
            s1 = p_dst[k + 3] + p_dst[k + 5];
            let s2 = fm(p_dst[k + 3] - p_dst[k + 5], C31);
            let r3 = p_dst[k + 1];
            p_dst[k + 1] = (r3 + s1) >> 2;
            s1 = r3 - (s1 >> 1);

            // combination
            p_dst[k + 2] = (r1 - s2) >> 2;
            p_dst[k + 4] = (r1 + s2) >> 2;
            p_dst[k + 3] = (s1 + r2) >> 2;
            p_dst[k + 5] = (s1 - r2) >> 2;

            k += 6;
        }
    }
    // Sort input vector for fft's of length 5
    // input5(0:4)   = [output3(0) output3(3) output3(6) output3(9) output3(12)];
    // input5(5:9)   = [output3(1) output3(4) output3(7) output3(10) output3(13)];
    // input5(10:14) = [output3(2) output3(5) output3(8) output3(11) output3(14)];
    // Merge 2 loops into one, brings about 10%
    {
        let p_src = &a_dst;
        let p_dst = &mut a_dst1;
        let mut k = 0usize;
        for i in 0..N3 {
            let l = 2 * i;
            p_dst[k] = p_src[l];
            p_dst[k + 1] = p_src[l + 1];
            p_dst[k + 2] = p_src[l + (2 * N3)];
            p_dst[k + 3] = p_src[l + 1 + (2 * N3)];
            p_dst[k + 4] = p_src[l + (4 * N3)];
            p_dst[k + 5] = p_src[l + 1 + (4 * N3)];
            p_dst[k + 6] = p_src[l + (6 * N3)];
            p_dst[k + 7] = p_src[l + 1 + (6 * N3)];
            p_dst[k + 8] = p_src[l + (8 * N3)];
            p_dst[k + 9] = p_src[l + 1 + (8 * N3)];
            fft5_w(&mut p_dst[k..]);
            k += 10;
        }
    }
    // Sort output vector of length 15
    // output = [out5(0)  out5(6)  out5(12) out5(3)  out5(9)
    //           out5(10) out5(1)  out5(7)  out5(13) out5(4)
    //           out5(5)  out5(11) out5(2)  out5(8)  out5(14)];
    // optimize clumsy loop, brings about 5%
    {
        let p_src = &a_dst1;
        let p_dst = p_input;
        let mut l = 0usize;
        let mut k = 0usize;
        for _ in 0..N3 {
            p_dst[k] = p_src[l];
            p_dst[k + 1] = p_src[l + 1];
            l += 2 * N6;
            if l >= 2 * N15 {
                l -= 2 * N15;
            }
            p_dst[k + 2] = p_src[l];
            p_dst[k + 3] = p_src[l + 1];
            l += 2 * N6;
            if l >= 2 * N15 {
                l -= 2 * N15;
            }
            p_dst[k + 4] = p_src[l];
            p_dst[k + 5] = p_src[l + 1];
            l += 2 * N6;
            if l >= 2 * N15 {
                l -= 2 * N15;
            }
            p_dst[k + 6] = p_src[l];
            p_dst[k + 7] = p_src[l + 1];
            l += 2 * N6;
            if l >= 2 * N15 {
                l -= 2 * N15;
            }
            p_dst[k + 8] = p_src[l];
            p_dst[k + 9] = p_src[l + 1];
            l += 2; // no modulo check needed, it cannot occur
            k += 10;
        }
    }
}

// ---------------------------------------------------------------------------------------------
// fft.cpp: fft_16, fft_32
// ---------------------------------------------------------------------------------------------

/// `fft16_w16[2]` (file-static in fft.cpp): the `(re, im)` pairs of
///
/// `scripts/dump_table.py libFDK/src/fft.cpp fft16_w16 SHORT --hex`
pub static FFT16_W16: [FixpStp; 2] = [FixpSpk::new(0x7642, 0x30FC), FixpSpk::new(0x30FC, 0x7642)];

/// `fft_16(x)`: in-place complex FFT of length 16 (radix 4 first stages, then radix 2).
///
/// `SHIFT_A`/`SHIFT_B` differ between the variants (see the module documentation).
///
/// On `aarch64` targets the `aarch64` variant runs a NEON version (`neon16::fft_16_arm`), on x86
/// targets the other variants an AVX2 version if available (`x86_16::fft_16`); the straight port
/// is [`fft_16_ref`].
pub fn fft_16(x: &mut [FixpDbl]) {
    #[cfg(target_arch = "aarch64")]
    if ARCH_ARM {
        return neon16::fft_16_arm(x);
    }
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    if !ARCH_ARM && crate::simd::avx2() {
        // SAFETY: AVX2 is available.
        return unsafe { x86_16::fft_16(x) };
    }
    fft_16_w(as_wrapping(x));
}

/// `fft_16(x)`: the straight port of [`fft_16`].
pub fn fft_16_ref(x: &mut [FixpDbl]) {
    fft_16_w(as_wrapping(x));
}

#[allow(unused_assignments)]
pub(crate) fn fft_16_w<D: Lane>(x: &mut [D]) {
    let x = &mut x[..32];
    let (mut vr, mut ur, mut vr2, mut ur2, mut vr3, mut ur3, mut vr4, mut ur4);
    let (mut vi, mut ui, mut vi2, mut ui2, mut vi3, mut ui3);

    vr = (x[0] >> 1) + (x[16] >> 1);
    ur = (x[1] >> 1) + (x[17] >> 1);
    vi = shift_a(x[8]) + shift_a(x[24]);
    ui = shift_a(x[9]) + shift_a(x[25]);
    x[0] = vr + shift_b(vi);
    x[1] = ur + shift_b(ui);
    vr2 = (x[4] >> 1) + (x[20] >> 1);
    ur2 = (x[5] >> 1) + (x[21] >> 1);
    x[4] = vr - shift_b(vi);
    x[5] = ur - shift_b(ui);
    vr -= x[16];
    vi = shift_b(vi) - x[24];
    ur -= x[17];
    ui = shift_b(ui) - x[25];
    vr3 = (x[2] >> 1) + (x[18] >> 1);
    ur3 = (x[3] >> 1) + (x[19] >> 1);
    x[2] = ui + vr;
    x[3] = ur - vi;
    vr4 = (x[6] >> 1) + (x[22] >> 1);
    ur4 = (x[7] >> 1) + (x[23] >> 1);
    x[6] = vr - ui;
    x[7] = vi + ur;
    vi2 = shift_a(x[12]) + shift_a(x[28]);
    ui2 = shift_a(x[13]) + shift_a(x[29]);
    x[8] = vr2 + shift_b(vi2);
    x[9] = ur2 + shift_b(ui2);
    x[12] = vr2 - shift_b(vi2);
    x[13] = ur2 - shift_b(ui2);
    vr2 -= x[20];
    ur2 -= x[21];
    vi2 = shift_b(vi2) - x[28];
    ui2 = shift_b(ui2) - x[29];
    vi = shift_a(x[10]) + shift_a(x[26]);
    ui = shift_a(x[11]) + shift_a(x[27]);
    x[10] = ui2 + vr2;
    x[11] = ur2 - vi2;
    vi3 = shift_a(x[14]) + shift_a(x[30]);
    ui3 = shift_a(x[15]) + shift_a(x[31]);
    x[14] = vr2 - ui2;
    x[15] = vi2 + ur2;
    x[16] = vr3 + shift_b(vi);
    x[17] = ur3 + shift_b(ui);
    x[20] = vr3 - shift_b(vi);
    x[21] = ur3 - shift_b(ui);
    vr3 -= x[18];
    ur3 -= x[19];
    vi = shift_b(vi) - x[26];
    ui = shift_b(ui) - x[27];
    x[18] = ui + vr3;
    x[19] = ur3 - vi;
    x[24] = vr4 + shift_b(vi3);
    x[28] = vr4 - shift_b(vi3);
    x[25] = ur4 + shift_b(ui3);
    x[29] = ur4 - shift_b(ui3);
    vr4 -= x[22];
    ur4 -= x[23];
    x[22] = vr3 - ui;
    x[23] = vi + ur3;
    vi3 = shift_b(vi3) - x[30];
    ui3 = shift_b(ui3) - x[31];
    x[26] = ui3 + vr4;
    x[30] = vr4 - ui3;
    x[27] = ur4 - vi3;
    x[31] = vi3 + ur4;
    vr = x[8];
    vi = x[9];
    ur = x[0] >> 1;
    ui = x[1] >> 1;
    x[0] = ur + (vr >> 1);
    x[1] = ui + (vi >> 1);
    x[8] = ur - (vr >> 1);
    x[9] = ui - (vi >> 1);
    vr = x[13];
    vi = x[12];
    ur = x[4] >> 1;
    ui = x[5] >> 1;
    x[4] = ur + (vr >> 1);
    x[5] = ui - (vi >> 1);
    x[12] = ur - (vr >> 1);
    x[13] = ui + (vi >> 1);
    vr = x[24];
    vi = x[25];
    ur = x[16] >> 1;
    ui = x[17] >> 1;
    x[16] = ur + (vr >> 1);
    x[17] = ui + (vi >> 1);
    x[24] = ur - (vr >> 1);
    x[25] = ui - (vi >> 1);
    vr = x[29];
    vi = x[28];
    ur = x[20] >> 1;
    ui = x[21] >> 1;
    x[20] = ur + (vr >> 1);
    x[21] = ui - (vi >> 1);
    x[28] = ur - (vr >> 1);
    x[29] = ui + (vi >> 1);
    (vi, vr) = sumdiff_pifourth(x[10], x[11]);
    ur = x[2];
    ui = x[3];
    x[2] = (ur >> 1) + vr;
    x[3] = (ui >> 1) + vi;
    x[10] = (ur >> 1) - vr;
    x[11] = (ui >> 1) - vi;
    (vr, vi) = sumdiff_pifourth(x[14], x[15]);
    ur = x[6];
    ui = x[7];
    x[6] = (ur >> 1) + vr;
    x[7] = (ui >> 1) - vi;
    x[14] = (ur >> 1) - vr;
    x[15] = (ui >> 1) + vi;
    (vi, vr) = sumdiff_pifourth(x[26], x[27]);
    ur = x[18];
    ui = x[19];
    x[18] = (ur >> 1) + vr;
    x[19] = (ui >> 1) + vi;
    x[26] = (ur >> 1) - vr;
    x[27] = (ui >> 1) - vi;
    (vr, vi) = sumdiff_pifourth(x[30], x[31]);
    ur = x[22];
    ui = x[23];
    x[22] = (ur >> 1) + vr;
    x[23] = (ui >> 1) - vi;
    x[30] = (ur >> 1) - vr;
    x[31] = (ui >> 1) + vi;
    vr = x[16];
    vi = x[17];
    ur = x[0] >> 1;
    ui = x[1] >> 1;
    x[0] = ur + (vr >> 1);
    x[1] = ui + (vi >> 1);
    x[16] = ur - (vr >> 1);
    x[17] = ui - (vi >> 1);
    vi = x[24];
    vr = x[25];
    ur = x[8] >> 1;
    ui = x[9] >> 1;
    x[8] = ur + (vr >> 1);
    x[9] = ui - (vi >> 1);
    x[24] = ur - (vr >> 1);
    x[25] = ui + (vi >> 1);
    (vi, vr) = cmd2(x[19], x[18], FFT16_W16[0]);
    ur = x[2];
    ui = x[3];
    x[2] = (ur >> 1) + vr;
    x[3] = (ui >> 1) + vi;
    x[18] = (ur >> 1) - vr;
    x[19] = (ui >> 1) - vi;
    (vr, vi) = cmd2(x[27], x[26], FFT16_W16[0]);
    ur = x[10];
    ui = x[11];
    x[10] = (ur >> 1) + vr;
    x[11] = (ui >> 1) - vi;
    x[26] = (ur >> 1) - vr;
    x[27] = (ui >> 1) + vi;
    (vi, vr) = sumdiff_pifourth(x[20], x[21]);
    ur = x[4];
    ui = x[5];
    x[4] = (ur >> 1) + vr;
    x[5] = (ui >> 1) + vi;
    x[20] = (ur >> 1) - vr;
    x[21] = (ui >> 1) - vi;
    (vr, vi) = sumdiff_pifourth(x[28], x[29]);
    ur = x[12];
    ui = x[13];
    x[12] = (ur >> 1) + vr;
    x[13] = (ui >> 1) - vi;
    x[28] = (ur >> 1) - vr;
    x[29] = (ui >> 1) + vi;
    (vi, vr) = cmd2(x[23], x[22], FFT16_W16[1]);
    ur = x[6];
    ui = x[7];
    x[6] = (ur >> 1) + vr;
    x[7] = (ui >> 1) + vi;
    x[22] = (ur >> 1) - vr;
    x[23] = (ui >> 1) - vi;
    (vr, vi) = cmd2(x[31], x[30], FFT16_W16[1]);
    ur = x[14];
    ui = x[15];
    x[14] = (ur >> 1) + vr;
    x[15] = (ui >> 1) - vi;
    x[30] = (ur >> 1) - vr;
    x[31] = (ui >> 1) + vi;
}

/// `fft32_w32[6]` (file-static in fft.cpp): the `(re, im)` pairs of
///
/// `scripts/dump_table.py libFDK/src/fft.cpp fft32_w32 SHORT --hex`
pub static FFT32_W32: [FixpStp; 6] = [
    FixpSpk::new(0x7642, 0x30FC),
    FixpSpk::new(0x30FC, 0x7642),
    FixpSpk::new(0x7D8A, 0x18F9),
    FixpSpk::new(0x6A6E, 0x471D),
    FixpSpk::new(0x471D, 0x6A6E),
    FixpSpk::new(0x18F9, 0x7D8A),
];

/// `fft_32(x)`: in-place complex FFT of length 32 (radix 4 first stages, then radix 2).
///
/// On `aarch64` targets the `aarch64` variant runs a NEON version (`neon16::fft_32_arm`), on x86
/// targets the other variants an AVX2 version if available (`x86_16::fft_32`); the straight port
/// is [`fft_32_ref`].
pub fn fft_32(x: &mut [FixpDbl]) {
    #[cfg(target_arch = "aarch64")]
    if ARCH_ARM {
        return neon16::fft_32_arm(x);
    }
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    if !ARCH_ARM && crate::simd::avx2() {
        // SAFETY: AVX2 is available.
        return unsafe { x86_16::fft_32(x) };
    }
    fft_32_w(as_wrapping(x));
}

/// `fft_32(x)`: the straight port of [`fft_32`].
pub fn fft_32_ref(x: &mut [FixpDbl]) {
    fft_32_w(as_wrapping(x));
}

#[allow(unused_assignments)]
pub(crate) fn fft_32_w<D: Lane>(x: &mut [D]) {
    let x = &mut x[..64];
    let (mut vr, mut ur, mut vr2, mut ur2, mut vr3, mut ur3, mut vr4, mut ur4);
    let (mut vi, mut ui, mut vi2, mut ui2, mut vi3, mut ui3);

    // 1+2 stage radix 4
    {
        vr = (x[0] + x[32]) >> 1;
        ur = (x[1] + x[33]) >> 1;
        vi = shift_a(x[16] + x[48]);
        ui = shift_a(x[17] + x[49]);
        x[0] = vr + shift_b(vi);
        x[1] = ur + shift_b(ui);
        vr2 = (x[4] + x[36]) >> 1;
        ur2 = (x[5] + x[37]) >> 1;
        x[4] = vr - shift_b(vi);
        x[5] = ur - shift_b(ui);
        vr -= x[32];
        ur -= x[33];
        vi = shift_b(vi) - x[48];
        ui = shift_b(ui) - x[49];
        vr3 = (x[2] + x[34]) >> 1;
        ur3 = (x[3] + x[35]) >> 1;
        x[2] = ui + vr;
        x[3] = ur - vi;
        vr4 = (x[6] + x[38]) >> 1;
        ur4 = (x[7] + x[39]) >> 1;
        x[6] = vr - ui;
        x[7] = vi + ur;
        vi = shift_a(x[20] + x[52]);
        ui = shift_a(x[21] + x[53]);
        x[16] = vr2 + shift_b(vi);
        x[17] = ur2 + shift_b(ui);
        x[20] = vr2 - shift_b(vi);
        x[21] = ur2 - shift_b(ui);
        vr2 -= x[36];
        ur2 -= x[37];
        vi = shift_b(vi) - x[52];
        ui = shift_b(ui) - x[53];
        vi2 = shift_a(x[18] + x[50]);
        ui2 = shift_a(x[19] + x[51]);
        x[18] = ui + vr2;
        x[19] = ur2 - vi;
        vi3 = shift_a(x[22] + x[54]);
        ui3 = shift_a(x[23] + x[55]);
        x[22] = vr2 - ui;
        x[23] = vi + ur2;
        x[32] = vr3 + shift_b(vi2);
        x[33] = ur3 + shift_b(ui2);
        x[36] = vr3 - shift_b(vi2);
        x[37] = ur3 - shift_b(ui2);
        vr3 -= x[34];
        ur3 -= x[35];
        vi2 = shift_b(vi2) - x[50];
        ui2 = shift_b(ui2) - x[51];
        x[34] = ui2 + vr3;
        x[35] = ur3 - vi2;
        x[48] = vr4 + shift_b(vi3);
        x[52] = vr4 - shift_b(vi3);
        x[49] = ur4 + shift_b(ui3);
        x[53] = ur4 - shift_b(ui3);
        vr4 -= x[38];
        ur4 -= x[39];
        x[38] = vr3 - ui2;
        x[39] = vi2 + ur3;
        vi3 = shift_b(vi3) - x[54];
        ui3 = shift_b(ui3) - x[55];
        x[50] = ui3 + vr4;
        x[54] = vr4 - ui3;
        x[51] = ur4 - vi3;
        x[55] = vi3 + ur4;
        vr = (x[8] + x[40]) >> 1;
        ur = (x[9] + x[41]) >> 1;
        vi = shift_a(x[24] + x[56]);
        ui = shift_a(x[25] + x[57]);
        x[8] = vr + shift_b(vi);
        x[9] = ur + shift_b(ui);
        vr2 = (x[12] + x[44]) >> 1;
        ur2 = (x[13] + x[45]) >> 1;
        x[12] = vr - shift_b(vi);
        x[13] = ur - shift_b(ui);
        vr -= x[40];
        ur -= x[41];
        vi = shift_b(vi) - x[56];
        ui = shift_b(ui) - x[57];
        vr3 = (x[10] + x[42]) >> 1;
        ur3 = (x[11] + x[43]) >> 1;
        x[10] = ui + vr;
        x[11] = ur - vi;
        vr4 = (x[14] + x[46]) >> 1;
        ur4 = (x[15] + x[47]) >> 1;
        x[14] = vr - ui;
        x[15] = vi + ur;
        vi = shift_a(x[28] + x[60]);
        ui = shift_a(x[29] + x[61]);
        x[24] = vr2 + shift_b(vi);
        x[28] = vr2 - shift_b(vi);
        x[25] = ur2 + shift_b(ui);
        x[29] = ur2 - shift_b(ui);
        vr2 -= x[44];
        ur2 -= x[45];
        vi = shift_b(vi) - x[60];
        ui = shift_b(ui) - x[61];
        vi2 = shift_a(x[26] + x[58]);
        ui2 = shift_a(x[27] + x[59]);
        x[26] = ui + vr2;
        x[27] = ur2 - vi;
        vi3 = shift_a(x[30] + x[62]);
        ui3 = shift_a(x[31] + x[63]);
        x[30] = vr2 - ui;
        x[31] = vi + ur2;
        x[40] = vr3 + shift_b(vi2);
        x[44] = vr3 - shift_b(vi2);
        x[41] = ur3 + shift_b(ui2);
        x[45] = ur3 - shift_b(ui2);
        vr3 -= x[42];
        ur3 -= x[43];
        vi2 = shift_b(vi2) - x[58];
        ui2 = shift_b(ui2) - x[59];
        x[42] = ui2 + vr3;
        x[43] = ur3 - vi2;
        x[56] = vr4 + shift_b(vi3);
        x[60] = vr4 - shift_b(vi3);
        x[57] = ur4 + shift_b(ui3);
        x[61] = ur4 - shift_b(ui3);
        vr4 -= x[46];
        ur4 -= x[47];
        x[46] = vr3 - ui2;
        x[47] = vi2 + ur3;
        vi3 = shift_b(vi3) - x[62];
        ui3 = shift_b(ui3) - x[63];
        x[58] = ui3 + vr4;
        x[62] = vr4 - ui3;
        x[59] = ur4 - vi3;
        x[63] = vi3 + ur4;
    }
    {
        let mut xt = 0usize;
        let mut j = 4;
        loop {
            vr = x[xt + 8];
            vi = x[xt + 9];
            ur = x[xt + 0] >> 1;
            ui = x[xt + 1] >> 1;
            x[xt + 0] = ur + (vr >> 1);
            x[xt + 1] = ui + (vi >> 1);
            x[xt + 8] = ur - (vr >> 1);
            x[xt + 9] = ui - (vi >> 1);
            vr = x[xt + 13];
            vi = x[xt + 12];
            ur = x[xt + 4] >> 1;
            ui = x[xt + 5] >> 1;
            x[xt + 4] = ur + (vr >> 1);
            x[xt + 5] = ui - (vi >> 1);
            x[xt + 12] = ur - (vr >> 1);
            x[xt + 13] = ui + (vi >> 1);
            (vi, vr) = sumdiff_pifourth(x[xt + 10], x[xt + 11]);
            ur = x[xt + 2];
            ui = x[xt + 3];
            x[xt + 2] = (ur >> 1) + vr;
            x[xt + 3] = (ui >> 1) + vi;
            x[xt + 10] = (ur >> 1) - vr;
            x[xt + 11] = (ui >> 1) - vi;
            (vr, vi) = sumdiff_pifourth(x[xt + 14], x[xt + 15]);
            ur = x[xt + 6];
            ui = x[xt + 7];
            x[xt + 6] = (ur >> 1) + vr;
            x[xt + 7] = (ui >> 1) - vi;
            x[xt + 14] = (ur >> 1) - vr;
            x[xt + 15] = (ui >> 1) + vi;
            xt += 16;
            j -= 1;
            if j == 0 {
                break;
            }
        }
    }
    {
        vr = x[16];
        vi = x[17];
        ur = x[0] >> 1;
        ui = x[1] >> 1;
        x[0] = ur + (vr >> 1);
        x[1] = ui + (vi >> 1);
        x[16] = ur - (vr >> 1);
        x[17] = ui - (vi >> 1);
        vi = x[24];
        vr = x[25];
        ur = x[8] >> 1;
        ui = x[9] >> 1;
        x[8] = ur + (vr >> 1);
        x[9] = ui - (vi >> 1);
        x[24] = ur - (vr >> 1);
        x[25] = ui + (vi >> 1);
        vr = x[48];
        vi = x[49];
        ur = x[32] >> 1;
        ui = x[33] >> 1;
        x[32] = ur + (vr >> 1);
        x[33] = ui + (vi >> 1);
        x[48] = ur - (vr >> 1);
        x[49] = ui - (vi >> 1);
        vi = x[56];
        vr = x[57];
        ur = x[40] >> 1;
        ui = x[41] >> 1;
        x[40] = ur + (vr >> 1);
        x[41] = ui - (vi >> 1);
        x[56] = ur - (vr >> 1);
        x[57] = ui + (vi >> 1);
        (vi, vr) = cmd2(x[19], x[18], FFT32_W32[0]);
        ur = x[2];
        ui = x[3];
        x[2] = (ur >> 1) + vr;
        x[3] = (ui >> 1) + vi;
        x[18] = (ur >> 1) - vr;
        x[19] = (ui >> 1) - vi;
        (vr, vi) = cmd2(x[27], x[26], FFT32_W32[0]);
        ur = x[10];
        ui = x[11];
        x[10] = (ur >> 1) + vr;
        x[11] = (ui >> 1) - vi;
        x[26] = (ur >> 1) - vr;
        x[27] = (ui >> 1) + vi;
        (vi, vr) = cmd2(x[51], x[50], FFT32_W32[0]);
        ur = x[34];
        ui = x[35];
        x[34] = (ur >> 1) + vr;
        x[35] = (ui >> 1) + vi;
        x[50] = (ur >> 1) - vr;
        x[51] = (ui >> 1) - vi;
        (vr, vi) = cmd2(x[59], x[58], FFT32_W32[0]);
        ur = x[42];
        ui = x[43];
        x[42] = (ur >> 1) + vr;
        x[43] = (ui >> 1) - vi;
        x[58] = (ur >> 1) - vr;
        x[59] = (ui >> 1) + vi;
        (vi, vr) = sumdiff_pifourth(x[20], x[21]);
        ur = x[4];
        ui = x[5];
        x[4] = (ur >> 1) + vr;
        x[5] = (ui >> 1) + vi;
        x[20] = (ur >> 1) - vr;
        x[21] = (ui >> 1) - vi;
        (vr, vi) = sumdiff_pifourth(x[28], x[29]);
        ur = x[12];
        ui = x[13];
        x[12] = (ur >> 1) + vr;
        x[13] = (ui >> 1) - vi;
        x[28] = (ur >> 1) - vr;
        x[29] = (ui >> 1) + vi;
        (vi, vr) = sumdiff_pifourth(x[52], x[53]);
        ur = x[36];
        ui = x[37];
        x[36] = (ur >> 1) + vr;
        x[37] = (ui >> 1) + vi;
        x[52] = (ur >> 1) - vr;
        x[53] = (ui >> 1) - vi;
        (vr, vi) = sumdiff_pifourth(x[60], x[61]);
        ur = x[44];
        ui = x[45];
        x[44] = (ur >> 1) + vr;
        x[45] = (ui >> 1) - vi;
        x[60] = (ur >> 1) - vr;
        x[61] = (ui >> 1) + vi;
        (vi, vr) = cmd2(x[23], x[22], FFT32_W32[1]);
        ur = x[6];
        ui = x[7];
        x[6] = (ur >> 1) + vr;
        x[7] = (ui >> 1) + vi;
        x[22] = (ur >> 1) - vr;
        x[23] = (ui >> 1) - vi;
        (vr, vi) = cmd2(x[31], x[30], FFT32_W32[1]);
        ur = x[14];
        ui = x[15];
        x[14] = (ur >> 1) + vr;
        x[15] = (ui >> 1) - vi;
        x[30] = (ur >> 1) - vr;
        x[31] = (ui >> 1) + vi;
        (vi, vr) = cmd2(x[55], x[54], FFT32_W32[1]);
        ur = x[38];
        ui = x[39];
        x[38] = (ur >> 1) + vr;
        x[39] = (ui >> 1) + vi;
        x[54] = (ur >> 1) - vr;
        x[55] = (ui >> 1) - vi;
        (vr, vi) = cmd2(x[63], x[62], FFT32_W32[1]);
        ur = x[46];
        ui = x[47];
        x[46] = (ur >> 1) + vr;
        x[47] = (ui >> 1) - vi;
        x[62] = (ur >> 1) - vr;
        x[63] = (ui >> 1) + vi;
        vr = x[32];
        vi = x[33];
        ur = x[0] >> 1;
        ui = x[1] >> 1;
        x[0] = ur + (vr >> 1);
        x[1] = ui + (vi >> 1);
        x[32] = ur - (vr >> 1);
        x[33] = ui - (vi >> 1);
        vi = x[48];
        vr = x[49];
        ur = x[16] >> 1;
        ui = x[17] >> 1;
        x[16] = ur + (vr >> 1);
        x[17] = ui - (vi >> 1);
        x[48] = ur - (vr >> 1);
        x[49] = ui + (vi >> 1);
        (vi, vr) = cmd2(x[35], x[34], FFT32_W32[2]);
        ur = x[2];
        ui = x[3];
        x[2] = (ur >> 1) + vr;
        x[3] = (ui >> 1) + vi;
        x[34] = (ur >> 1) - vr;
        x[35] = (ui >> 1) - vi;
        (vr, vi) = cmd2(x[51], x[50], FFT32_W32[2]);
        ur = x[18];
        ui = x[19];
        x[18] = (ur >> 1) + vr;
        x[19] = (ui >> 1) - vi;
        x[50] = (ur >> 1) - vr;
        x[51] = (ui >> 1) + vi;
        (vi, vr) = cmd2(x[37], x[36], FFT32_W32[0]);
        ur = x[4];
        ui = x[5];
        x[4] = (ur >> 1) + vr;
        x[5] = (ui >> 1) + vi;
        x[36] = (ur >> 1) - vr;
        x[37] = (ui >> 1) - vi;
        (vr, vi) = cmd2(x[53], x[52], FFT32_W32[0]);
        ur = x[20];
        ui = x[21];
        x[20] = (ur >> 1) + vr;
        x[21] = (ui >> 1) - vi;
        x[52] = (ur >> 1) - vr;
        x[53] = (ui >> 1) + vi;
        (vi, vr) = cmd2(x[39], x[38], FFT32_W32[3]);
        ur = x[6];
        ui = x[7];
        x[6] = (ur >> 1) + vr;
        x[7] = (ui >> 1) + vi;
        x[38] = (ur >> 1) - vr;
        x[39] = (ui >> 1) - vi;
        (vr, vi) = cmd2(x[55], x[54], FFT32_W32[3]);
        ur = x[22];
        ui = x[23];
        x[22] = (ur >> 1) + vr;
        x[23] = (ui >> 1) - vi;
        x[54] = (ur >> 1) - vr;
        x[55] = (ui >> 1) + vi;
        (vi, vr) = sumdiff_pifourth(x[40], x[41]);
        ur = x[8];
        ui = x[9];
        x[8] = (ur >> 1) + vr;
        x[9] = (ui >> 1) + vi;
        x[40] = (ur >> 1) - vr;
        x[41] = (ui >> 1) - vi;
        (vr, vi) = sumdiff_pifourth(x[56], x[57]);
        ur = x[24];
        ui = x[25];
        x[24] = (ur >> 1) + vr;
        x[25] = (ui >> 1) - vi;
        x[56] = (ur >> 1) - vr;
        x[57] = (ui >> 1) + vi;
        (vi, vr) = cmd2(x[43], x[42], FFT32_W32[4]);
        ur = x[10];
        ui = x[11];
        x[10] = (ur >> 1) + vr;
        x[11] = (ui >> 1) + vi;
        x[42] = (ur >> 1) - vr;
        x[43] = (ui >> 1) - vi;
        (vr, vi) = cmd2(x[59], x[58], FFT32_W32[4]);
        ur = x[26];
        ui = x[27];
        x[26] = (ur >> 1) + vr;
        x[27] = (ui >> 1) - vi;
        x[58] = (ur >> 1) - vr;
        x[59] = (ui >> 1) + vi;
        (vi, vr) = cmd2(x[45], x[44], FFT32_W32[1]);
        ur = x[12];
        ui = x[13];
        x[12] = (ur >> 1) + vr;
        x[13] = (ui >> 1) + vi;
        x[44] = (ur >> 1) - vr;
        x[45] = (ui >> 1) - vi;
        (vr, vi) = cmd2(x[61], x[60], FFT32_W32[1]);
        ur = x[28];
        ui = x[29];
        x[28] = (ur >> 1) + vr;
        x[29] = (ui >> 1) - vi;
        x[60] = (ur >> 1) - vr;
        x[61] = (ui >> 1) + vi;
        (vi, vr) = cmd2(x[47], x[46], FFT32_W32[5]);
        ur = x[14];
        ui = x[15];
        x[14] = (ur >> 1) + vr;
        x[15] = (ui >> 1) + vi;
        x[46] = (ur >> 1) - vr;
        x[47] = (ui >> 1) - vi;
        (vr, vi) = cmd2(x[63], x[62], FFT32_W32[5]);
        ur = x[30];
        ui = x[31];
        x[30] = (ur >> 1) + vr;
        x[31] = (ui >> 1) - vi;
        x[62] = (ur >> 1) - vr;
        x[63] = (ui >> 1) + vi;
    }
}

#[cfg(target_arch = "aarch64")]
mod neon;
#[cfg(target_arch = "aarch64")]
pub(crate) mod neon16;
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
pub(crate) mod x86;
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
pub(crate) mod x86_16;

// ---------------------------------------------------------------------------------------------
// fft.cpp: two-dimensional FFTs
// ---------------------------------------------------------------------------------------------

/// `fft_apply_rot_vector(pData, cl, l, pVecRe, pVecIm)`: applies the rotation vectors to a data
/// buffer of `l` complex values in rows of `cl` values.
fn fft_apply_rot_vector<D: Lane>(
    p_data: &mut [D],
    cl: usize,
    l: usize,
    p_vec_re: &[FixpStb],
    p_vec_im: &[FixpStb],
) {
    let mut i = 0usize;
    while i < cl {
        let re = p_data[2 * i];
        let im = p_data[2 * i + 1];

        p_data[2 * i] = re >> 2; // * 0.25
        p_data[2 * i + 1] = im >> 2; // * 0.25
        i += 1;
    }
    let mut v = 0usize;
    while i < l {
        let re = p_data[2 * i];
        let im = p_data[2 * i + 1];

        p_data[2 * i] = re >> 2; // * 0.25
        p_data[2 * i + 1] = im >> 2; // * 0.25

        for c in i + 1..i + cl {
            let re = p_data[2 * c] >> 1;
            let im = p_data[2 * c + 1] >> 1;
            let vre = p_vec_re[v];
            let vim = p_vec_im[v];
            v += 1;

            (p_data[2 * c + 1], p_data[2 * c]) = cmd2_s(im, re, vre, vim);
        }
        i += cl;
    }
}

/// `fftN2_func(pInput, length, dim1, dim2, fft1, fft2, RotVectorReal, RotVectorImag, aDst,
/// aDst2)` with the scratch buffers of the `fftN2` macro: `dim2` FFTs of length `dim1`, the
/// rotation, `dim1` FFTs of length `dim2`.
#[allow(clippy::too_many_arguments)]
fn fftN2<D: Lane>(
    p_input: &mut [D],
    length: usize,
    dim1: usize,
    dim2: usize,
    fft1: fn(&mut [D]),
    fft2: fn(&mut [D]),
    rot_vector_real: &[FixpStb],
    rot_vector_imag: &[FixpStb],
) {
    fdk_assert!(length == dim1 * dim2);
    // C_AALLOC_SCRATCH_START(aDst, DATA_TYPE, 2 * length) and (aDst2, DATA_TYPE, 2 * dim2):
    // every value is written before it is read.
    let mut a_dst_buf = [D::ZERO; 2 * 480];
    let mut a_dst2_buf = [D::ZERO; 2 * 32];
    let a_dst = &mut a_dst_buf[..2 * length];
    let a_dst2 = &mut a_dst2_buf[..2 * dim2];

    // Perform dim2 times the fft of length dim1. The input samples are at the address of pSrc
    // and the output samples are at the address of pDst. The input vector for the fft of length
    // dim1 is built of the interleaved samples in pSrc, the output samples are stored
    // consecutively.
    let mut p_src = 0usize;
    let mut p_dst = 0usize;
    for _ in 0..dim2 {
        for j in 0..dim1 {
            a_dst[p_dst + 2 * j] = p_input[p_src + 2 * j * dim2];
            a_dst[p_dst + 2 * j + 1] = p_input[p_src + 2 * j * dim2 + 1];
        }

        // fft of size dim1
        fft1(&mut a_dst[p_dst..]);

        p_src += 2;
        p_dst += 2 * dim1;
    }

    // Perform the modulation of the output of the fft of length dim1
    fft_apply_rot_vector(a_dst, dim1, length, rot_vector_real, rot_vector_imag);

    // Perform dim1 times the fft of length dim2. The input samples are at the address of aDst
    // and the output samples are at the address of pInput. The input vector for the fft of
    // length dim2 is built of the interleaved samples in aDst, the output samples are stored
    // consecutively at the address of pInput.
    let mut p_src = 0usize;
    let mut p_dst_out = 0usize;
    for _ in 0..dim1 {
        for j in 0..dim2 {
            a_dst2[2 * j] = a_dst[p_src + 2 * j * dim1];
            a_dst2[2 * j + 1] = a_dst[p_src + 2 * j * dim1 + 1];
        }

        fft2(a_dst2);

        for j in 0..dim2 {
            p_input[p_dst_out + 2 * j * dim1] = a_dst2[2 * j];
            p_input[p_dst_out + 2 * j * dim1 + 1] = a_dst2[2 * j + 1];
        }
        p_src += 2;
        p_dst_out += 2;
    }
}

/// `fft6(pInput)`: `fftN2(..., 6, 2, 3, fft2, fft3, RotVectorReal6, RotVectorImag6)`
pub fn fft6(p_input: &mut [FixpDbl]) {
    fft6_g::<true>(p_input)
}

/// [`fft6`] (`FAST`: the transforms of a phase four at a time on `aarch64` targets, see
/// `neon::fft_n2_x4`, and for the other variants on x86 targets, see `x86::fft_n2_x4`) or the straight port.
#[inline(always)]
fn fft6_g<const FAST: bool>(p_input: &mut [FixpDbl]) {
    #[cfg(target_arch = "aarch64")]
    if FAST {
        return neon::fft_n2_x4(
            as_wrapping(p_input),
            6,
            2,
            3,
            fft2_w::<neon::V4>,
            fft3_w::<neon::V4>,
            fft2_w::<W>,
            fft3_w::<W>,
            &ROT_VECTOR_REAL_6,
            &ROT_VECTOR_IMAG_6,
        );
    }
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    if FAST && !ARCH_ARM {
        return x86::fft_n2_x4(
            as_wrapping(p_input),
            6,
            2,
            3,
            fft2_w::<x86::V4>,
            fft3_w::<x86::V4>,
            fft2_w::<W>,
            fft3_w::<W>,
            &ROT_VECTOR_REAL_6,
            &ROT_VECTOR_IMAG_6,
        );
    }
    fftN2(
        as_wrapping(p_input),
        6,
        2,
        3,
        fft2_w::<W>,
        fft3_w::<W>,
        &ROT_VECTOR_REAL_6,
        &ROT_VECTOR_IMAG_6,
    );
}

/// `fft20(pInput)`: `fftN2(..., 20, 4, 5, fft_4, fft5, RotVectorReal20, RotVectorImag20)`
pub fn fft20(p_input: &mut [FixpDbl]) {
    fft20_g::<true>(p_input)
}

/// [`fft20`] (`FAST`: the transforms of a phase four at a time on `aarch64` targets, see
/// `neon::fft_n2_x4`, and for the other variants on x86 targets, see `x86::fft_n2_x4`) or the straight port.
#[inline(always)]
fn fft20_g<const FAST: bool>(p_input: &mut [FixpDbl]) {
    #[cfg(target_arch = "aarch64")]
    if FAST {
        return neon::fft_n2_x4(
            as_wrapping(p_input),
            20,
            4,
            5,
            fft_4_w::<neon::V4>,
            fft5_w::<neon::V4>,
            fft_4_w::<W>,
            fft5_w::<W>,
            &ROT_VECTOR_REAL_20,
            &ROT_VECTOR_IMAG_20,
        );
    }
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    if FAST && !ARCH_ARM {
        return x86::fft_n2_x4(
            as_wrapping(p_input),
            20,
            4,
            5,
            fft_4_w::<x86::V4>,
            fft5_w::<x86::V4>,
            fft_4_w::<W>,
            fft5_w::<W>,
            &ROT_VECTOR_REAL_20,
            &ROT_VECTOR_IMAG_20,
        );
    }
    fftN2(
        as_wrapping(p_input),
        20,
        4,
        5,
        fft_4_w::<W>,
        fft5_w::<W>,
        &ROT_VECTOR_REAL_20,
        &ROT_VECTOR_IMAG_20,
    );
}

/// `fft24(pInput)`: `fftN2(..., 24, 2, 12, fft2, fft12, RotVectorReal24, RotVectorImag24)`
pub fn fft24(p_input: &mut [FixpDbl]) {
    fft24_g::<true>(p_input)
}

/// [`fft24`] (`FAST`: the transforms of a phase four at a time on `aarch64` targets, see
/// `neon::fft_n2_x4`, and for the other variants on x86 targets, see `x86::fft_n2_x4`) or the straight port.
#[inline(always)]
fn fft24_g<const FAST: bool>(p_input: &mut [FixpDbl]) {
    #[cfg(target_arch = "aarch64")]
    if FAST {
        return neon::fft_n2_x4(
            as_wrapping(p_input),
            24,
            2,
            12,
            fft2_w::<neon::V4>,
            fft12_w::<neon::V4>,
            fft2_w::<W>,
            fft12_w::<W>,
            &ROT_VECTOR_REAL_24,
            &ROT_VECTOR_IMAG_24,
        );
    }
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    if FAST && !ARCH_ARM {
        return x86::fft_n2_x4(
            as_wrapping(p_input),
            24,
            2,
            12,
            fft2_w::<x86::V4>,
            fft12_w::<x86::V4>,
            fft2_w::<W>,
            fft12_w::<W>,
            &ROT_VECTOR_REAL_24,
            &ROT_VECTOR_IMAG_24,
        );
    }
    fftN2(
        as_wrapping(p_input),
        24,
        2,
        12,
        fft2_w::<W>,
        fft12_w::<W>,
        &ROT_VECTOR_REAL_24,
        &ROT_VECTOR_IMAG_24,
    );
}

/// `fft48(pInput)`: `fftN2(..., 48, 4, 12, fft_4, fft12, RotVectorReal48, RotVectorImag48)`
pub fn fft48(p_input: &mut [FixpDbl]) {
    fft48_g::<true>(p_input)
}

/// [`fft48`] (`FAST`: the transforms of a phase four at a time on `aarch64` targets, see
/// `neon::fft_n2_x4`, and for the other variants on x86 targets, see `x86::fft_n2_x4`) or the straight port.
#[inline(always)]
fn fft48_g<const FAST: bool>(p_input: &mut [FixpDbl]) {
    #[cfg(target_arch = "aarch64")]
    if FAST {
        return neon::fft_n2_x4(
            as_wrapping(p_input),
            48,
            4,
            12,
            fft_4_w::<neon::V4>,
            fft12_w::<neon::V4>,
            fft_4_w::<W>,
            fft12_w::<W>,
            &ROT_VECTOR_REAL_48,
            &ROT_VECTOR_IMAG_48,
        );
    }
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    if FAST && !ARCH_ARM {
        return x86::fft_n2_x4(
            as_wrapping(p_input),
            48,
            4,
            12,
            fft_4_w::<x86::V4>,
            fft12_w::<x86::V4>,
            fft_4_w::<W>,
            fft12_w::<W>,
            &ROT_VECTOR_REAL_48,
            &ROT_VECTOR_IMAG_48,
        );
    }
    fftN2(
        as_wrapping(p_input),
        48,
        4,
        12,
        fft_4_w::<W>,
        fft12_w::<W>,
        &ROT_VECTOR_REAL_48,
        &ROT_VECTOR_IMAG_48,
    );
}

/// `fft60(pInput)`: `fftN2(..., 60, 4, 15, fft_4, fft15, RotVectorReal60, RotVectorImag60)`
pub fn fft60(p_input: &mut [FixpDbl]) {
    fft60_g::<true>(p_input)
}

/// [`fft60`] (`FAST`: the transforms of a phase four at a time on `aarch64` targets, see
/// `neon::fft_n2_x4`, and for the other variants on x86 targets, see `x86::fft_n2_x4`) or the straight port.
#[inline(always)]
fn fft60_g<const FAST: bool>(p_input: &mut [FixpDbl]) {
    #[cfg(target_arch = "aarch64")]
    if FAST {
        return neon::fft_n2_x4(
            as_wrapping(p_input),
            60,
            4,
            15,
            fft_4_w::<neon::V4>,
            fft15_w::<neon::V4>,
            fft_4_w::<W>,
            fft15_w::<W>,
            &ROT_VECTOR_REAL_60,
            &ROT_VECTOR_IMAG_60,
        );
    }
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    if FAST && !ARCH_ARM {
        return x86::fft_n2_x4(
            as_wrapping(p_input),
            60,
            4,
            15,
            fft_4_w::<x86::V4>,
            fft15_w::<x86::V4>,
            fft_4_w::<W>,
            fft15_w::<W>,
            &ROT_VECTOR_REAL_60,
            &ROT_VECTOR_IMAG_60,
        );
    }
    fftN2(
        as_wrapping(p_input),
        60,
        4,
        15,
        fft_4_w::<W>,
        fft15_w::<W>,
        &ROT_VECTOR_REAL_60,
        &ROT_VECTOR_IMAG_60,
    );
}

/// `fft80(pInput)`: `fftN2(..., 80, 5, 16, fft5, fft_16, RotVectorReal80, RotVectorImag80)`
pub fn fft80(p_input: &mut [FixpDbl]) {
    fft80_g::<true>(p_input)
}

/// [`fft80`] (`FAST`: the transforms of a phase four at a time on `aarch64` targets, see
/// `neon::fft_n2_x4`, and for the other variants on x86 targets, see `x86::fft_n2_x4`) or the straight port.
#[inline(always)]
fn fft80_g<const FAST: bool>(p_input: &mut [FixpDbl]) {
    #[cfg(target_arch = "aarch64")]
    if FAST {
        return neon::fft_n2_x4(
            as_wrapping(p_input),
            80,
            5,
            16,
            fft5_w::<neon::V4>,
            fft_16_w::<neon::V4>,
            fft5_w::<W>,
            fft_16_w::<W>,
            &ROT_VECTOR_REAL_80,
            &ROT_VECTOR_IMAG_80,
        );
    }
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    if FAST && !ARCH_ARM {
        return x86::fft_n2_x4(
            as_wrapping(p_input),
            80,
            5,
            16,
            fft5_w::<x86::V4>,
            fft_16_w::<x86::V4>,
            fft5_w::<W>,
            fft_16_w::<W>,
            &ROT_VECTOR_REAL_80,
            &ROT_VECTOR_IMAG_80,
        );
    }
    fftN2(
        as_wrapping(p_input),
        80,
        5,
        16,
        fft5_w::<W>,
        fft_16_w::<W>,
        &ROT_VECTOR_REAL_80,
        &ROT_VECTOR_IMAG_80,
    );
}

/// `fft96(pInput)`: `fftN2(..., 96, 3, 32, fft3, fft_32, RotVectorReal96, RotVectorImag96)`
pub fn fft96(p_input: &mut [FixpDbl]) {
    fft96_g::<true>(p_input)
}

/// [`fft96`] (`FAST`: the transforms of a phase four at a time on `aarch64` targets, see
/// `neon::fft_n2_x4`, and for the other variants on x86 targets, see `x86::fft_n2_x4`) or the straight port.
#[inline(always)]
fn fft96_g<const FAST: bool>(p_input: &mut [FixpDbl]) {
    #[cfg(target_arch = "aarch64")]
    if FAST {
        return neon::fft_n2_x4(
            as_wrapping(p_input),
            96,
            3,
            32,
            fft3_w::<neon::V4>,
            fft_32_w::<neon::V4>,
            fft3_w::<W>,
            fft_32_w::<W>,
            &ROT_VECTOR_REAL_96,
            &ROT_VECTOR_IMAG_96,
        );
    }
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    if FAST && !ARCH_ARM {
        return x86::fft_n2_x4(
            as_wrapping(p_input),
            96,
            3,
            32,
            fft3_w::<x86::V4>,
            fft_32_w::<x86::V4>,
            fft3_w::<W>,
            fft_32_w::<W>,
            &ROT_VECTOR_REAL_96,
            &ROT_VECTOR_IMAG_96,
        );
    }
    fftN2(
        as_wrapping(p_input),
        96,
        3,
        32,
        fft3_w::<W>,
        fft_32_w::<W>,
        &ROT_VECTOR_REAL_96,
        &ROT_VECTOR_IMAG_96,
    );
}

/// `fft120(pInput)`: `fftN2(..., 120, 8, 15, fft_8, fft15, RotVectorReal120, RotVectorImag120)`
pub fn fft120(p_input: &mut [FixpDbl]) {
    fft120_g::<true>(p_input)
}

/// [`fft120`] (`FAST`: the transforms of a phase four at a time on `aarch64` targets, see
/// `neon::fft_n2_x4`, and for the other variants on x86 targets, see `x86::fft_n2_x4`) or the straight port.
#[inline(always)]
fn fft120_g<const FAST: bool>(p_input: &mut [FixpDbl]) {
    #[cfg(target_arch = "aarch64")]
    if FAST {
        return neon::fft_n2_x4(
            as_wrapping(p_input),
            120,
            8,
            15,
            fft_8_w::<neon::V4>,
            fft15_w::<neon::V4>,
            fft_8_w::<W>,
            fft15_w::<W>,
            &ROT_VECTOR_REAL_120,
            &ROT_VECTOR_IMAG_120,
        );
    }
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    if FAST && !ARCH_ARM {
        return x86::fft_n2_x4(
            as_wrapping(p_input),
            120,
            8,
            15,
            fft_8_w::<x86::V4>,
            fft15_w::<x86::V4>,
            fft_8_w::<W>,
            fft15_w::<W>,
            &ROT_VECTOR_REAL_120,
            &ROT_VECTOR_IMAG_120,
        );
    }
    fftN2(
        as_wrapping(p_input),
        120,
        8,
        15,
        fft_8_w::<W>,
        fft15_w::<W>,
        &ROT_VECTOR_REAL_120,
        &ROT_VECTOR_IMAG_120,
    );
}

/// `fft192(pInput)`: `fftN2(..., 192, 16, 12, fft_16, fft12, RotVectorReal192, RotVectorImag192)`
pub fn fft192(p_input: &mut [FixpDbl]) {
    fft192_g::<true>(p_input)
}

/// [`fft192`] (`FAST`: the transforms of a phase four at a time on `aarch64` targets, see
/// `neon::fft_n2_x4`, and for the other variants on x86 targets, see `x86::fft_n2_x4`) or the straight port.
#[inline(always)]
fn fft192_g<const FAST: bool>(p_input: &mut [FixpDbl]) {
    #[cfg(target_arch = "aarch64")]
    if FAST {
        return neon::fft_n2_x4(
            as_wrapping(p_input),
            192,
            16,
            12,
            fft_16_w::<neon::V4>,
            fft12_w::<neon::V4>,
            fft_16_w::<W>,
            fft12_w::<W>,
            &ROT_VECTOR_REAL_192,
            &ROT_VECTOR_IMAG_192,
        );
    }
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    if FAST && !ARCH_ARM {
        return x86::fft_n2_x4(
            as_wrapping(p_input),
            192,
            16,
            12,
            fft_16_w::<x86::V4>,
            fft12_w::<x86::V4>,
            fft_16_w::<W>,
            fft12_w::<W>,
            &ROT_VECTOR_REAL_192,
            &ROT_VECTOR_IMAG_192,
        );
    }
    fftN2(
        as_wrapping(p_input),
        192,
        16,
        12,
        fft_16_w::<W>,
        fft12_w::<W>,
        &ROT_VECTOR_REAL_192,
        &ROT_VECTOR_IMAG_192,
    );
}

/// `fft240(pInput)`: `fftN2(..., 240, 16, 15, fft_16, fft15, RotVectorReal240, RotVectorImag240)`
pub fn fft240(p_input: &mut [FixpDbl]) {
    fft240_g::<true>(p_input)
}

/// [`fft240`] (`FAST`: the transforms of a phase four at a time on `aarch64` targets, see
/// `neon::fft_n2_x4`, and for the other variants on x86 targets, see `x86::fft_n2_x4`) or the straight port.
#[inline(always)]
fn fft240_g<const FAST: bool>(p_input: &mut [FixpDbl]) {
    #[cfg(target_arch = "aarch64")]
    if FAST {
        return neon::fft_n2_x4(
            as_wrapping(p_input),
            240,
            16,
            15,
            fft_16_w::<neon::V4>,
            fft15_w::<neon::V4>,
            fft_16_w::<W>,
            fft15_w::<W>,
            &ROT_VECTOR_REAL_240,
            &ROT_VECTOR_IMAG_240,
        );
    }
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    if FAST && !ARCH_ARM {
        return x86::fft_n2_x4(
            as_wrapping(p_input),
            240,
            16,
            15,
            fft_16_w::<x86::V4>,
            fft15_w::<x86::V4>,
            fft_16_w::<W>,
            fft15_w::<W>,
            &ROT_VECTOR_REAL_240,
            &ROT_VECTOR_IMAG_240,
        );
    }
    fftN2(
        as_wrapping(p_input),
        240,
        16,
        15,
        fft_16_w::<W>,
        fft15_w::<W>,
        &ROT_VECTOR_REAL_240,
        &ROT_VECTOR_IMAG_240,
    );
}

/// `fft384(pInput)`: `fftN2(..., 384, 12, 32, fft12, fft_32, RotVectorReal384, RotVectorImag384)`
pub fn fft384(p_input: &mut [FixpDbl]) {
    fft384_g::<true>(p_input)
}

/// [`fft384`] (`FAST`: the transforms of a phase four at a time on `aarch64` targets, see
/// `neon::fft_n2_x4`, and for the other variants on x86 targets, see `x86::fft_n2_x4`) or the straight port.
#[inline(always)]
fn fft384_g<const FAST: bool>(p_input: &mut [FixpDbl]) {
    #[cfg(target_arch = "aarch64")]
    if FAST {
        return neon::fft_n2_x4(
            as_wrapping(p_input),
            384,
            12,
            32,
            fft12_w::<neon::V4>,
            fft_32_w::<neon::V4>,
            fft12_w::<W>,
            fft_32_w::<W>,
            &ROT_VECTOR_REAL_384,
            &ROT_VECTOR_IMAG_384,
        );
    }
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    if FAST && !ARCH_ARM {
        return x86::fft_n2_x4(
            as_wrapping(p_input),
            384,
            12,
            32,
            fft12_w::<x86::V4>,
            fft_32_w::<x86::V4>,
            fft12_w::<W>,
            fft_32_w::<W>,
            &ROT_VECTOR_REAL_384,
            &ROT_VECTOR_IMAG_384,
        );
    }
    fftN2(
        as_wrapping(p_input),
        384,
        12,
        32,
        fft12_w::<W>,
        fft_32_w::<W>,
        &ROT_VECTOR_REAL_384,
        &ROT_VECTOR_IMAG_384,
    );
}

/// `fft480(pInput)`: `fftN2(..., 480, 32, 15, fft_32, fft15, RotVectorReal480, RotVectorImag480)`
pub fn fft480(p_input: &mut [FixpDbl]) {
    fft480_g::<true>(p_input)
}

/// [`fft480`] (`FAST`: the transforms of a phase four at a time on `aarch64` targets, see
/// `neon::fft_n2_x4`, and for the other variants on x86 targets, see `x86::fft_n2_x4`) or the straight port.
#[inline(always)]
fn fft480_g<const FAST: bool>(p_input: &mut [FixpDbl]) {
    #[cfg(target_arch = "aarch64")]
    if FAST {
        return neon::fft_n2_x4(
            as_wrapping(p_input),
            480,
            32,
            15,
            fft_32_w::<neon::V4>,
            fft15_w::<neon::V4>,
            fft_32_w::<W>,
            fft15_w::<W>,
            &ROT_VECTOR_REAL_480,
            &ROT_VECTOR_IMAG_480,
        );
    }
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    if FAST && !ARCH_ARM {
        return x86::fft_n2_x4(
            as_wrapping(p_input),
            480,
            32,
            15,
            fft_32_w::<x86::V4>,
            fft15_w::<x86::V4>,
            fft_32_w::<W>,
            fft15_w::<W>,
            &ROT_VECTOR_REAL_480,
            &ROT_VECTOR_IMAG_480,
        );
    }
    fftN2(
        as_wrapping(p_input),
        480,
        32,
        15,
        fft_32_w::<W>,
        fft15_w::<W>,
        &ROT_VECTOR_REAL_480,
        &ROT_VECTOR_IMAG_480,
    );
}

// ---------------------------------------------------------------------------------------------
// fft(), ifft()
// ---------------------------------------------------------------------------------------------

/// `fft(length, pInput, pScalefactor)`: in-place complex FFT of `length` interleaved complex
/// values; adds the scaling of the FFT to `*p_scalefactor`.
///
/// The input must have one bit of headroom. Unsupported lengths leave the data and the exponent
/// unchanged (`FDK_ASSERT(0)` in C, an `fdk_assert!` here).
///
/// The same as the straight port [`fft_ref`]: the lengths built with `fftN2` run four of the
/// transforms of each phase at a time on `aarch64` targets, the radix-2 lengths use the NEON
/// [`dit_fft`] of the `aarch64` variant.
pub fn fft(length: i32, p_input: &mut [FixpDbl], p_scalefactor: &mut i32) {
    fft_g::<true>(length, p_input, p_scalefactor)
}

/// `fft(length, pInput, pScalefactor)`: the straight port of [`fft`] (the straight `dit_fft`
/// and `fftN2`).
pub fn fft_ref(length: i32, p_input: &mut [FixpDbl], p_scalefactor: &mut i32) {
    fft_g::<false>(length, p_input, p_scalefactor)
}

#[inline(always)]
fn fft_g<const FAST: bool>(length: i32, p_input: &mut [FixpDbl], p_scalefactor: &mut i32) {
    if length == 32 {
        if FAST {
            fft_32(p_input);
        } else {
            fft_32_w(as_wrapping(p_input));
        }
        *p_scalefactor += SCALEFACTOR32;
    } else {
        match length {
            16 => {
                if FAST {
                    fft_16(p_input);
                } else {
                    fft_16_w(as_wrapping(p_input));
                }
                *p_scalefactor += SCALEFACTOR16;
            }
            8 => {
                fft_8(p_input);
                *p_scalefactor += SCALEFACTOR8;
            }
            2 => {
                fft2(p_input);
                *p_scalefactor += SCALEFACTOR2;
            }
            3 => {
                fft3(p_input);
                *p_scalefactor += SCALEFACTOR3;
            }
            4 => {
                fft_4(p_input);
                *p_scalefactor += SCALEFACTOR4;
            }
            5 => {
                fft5(p_input);
                *p_scalefactor += SCALEFACTOR5;
            }
            6 => {
                fft6_g::<FAST>(p_input);
                *p_scalefactor += SCALEFACTOR6;
            }
            10 => {
                fft10(p_input);
                *p_scalefactor += SCALEFACTOR10;
            }
            12 => {
                fft12(p_input);
                *p_scalefactor += SCALEFACTOR12;
            }
            15 => {
                fft15(p_input);
                *p_scalefactor += SCALEFACTOR15;
            }
            20 => {
                fft20_g::<FAST>(p_input);
                *p_scalefactor += SCALEFACTOR20;
            }
            24 => {
                fft24_g::<FAST>(p_input);
                *p_scalefactor += SCALEFACTOR24;
            }
            48 => {
                fft48_g::<FAST>(p_input);
                *p_scalefactor += SCALEFACTOR48;
            }
            60 => {
                fft60_g::<FAST>(p_input);
                *p_scalefactor += SCALEFACTOR60;
            }
            64 => {
                dit_fft_g::<FAST>(p_input, 6);
                *p_scalefactor += SCALEFACTOR64;
            }
            80 => {
                fft80_g::<FAST>(p_input);
                *p_scalefactor += SCALEFACTOR80;
            }
            96 => {
                fft96_g::<FAST>(p_input);
                *p_scalefactor += SCALEFACTOR96;
            }
            120 => {
                fft120_g::<FAST>(p_input);
                *p_scalefactor += SCALEFACTOR120;
            }
            128 => {
                dit_fft_g::<FAST>(p_input, 7);
                *p_scalefactor += SCALEFACTOR128;
            }
            192 => {
                fft192_g::<FAST>(p_input);
                *p_scalefactor += SCALEFACTOR192;
            }
            240 => {
                fft240_g::<FAST>(p_input);
                *p_scalefactor += SCALEFACTOR240;
            }
            256 => {
                dit_fft_g::<FAST>(p_input, 8);
                *p_scalefactor += SCALEFACTOR256;
            }
            384 => {
                fft384_g::<FAST>(p_input);
                *p_scalefactor += SCALEFACTOR384;
            }
            480 => {
                fft480_g::<FAST>(p_input);
                *p_scalefactor += SCALEFACTOR480;
            }
            512 => {
                dit_fft_g::<FAST>(p_input, 9);
                *p_scalefactor += SCALEFACTOR512;
            }
            _ => {
                fdk_assert!(false, "FFT length not supported: {length}");
            }
        }
    }
}

/// `dit_fft(x, ldn, SineTable512, 512)`: [`dit_fft`] (`FAST`) or the straight port of the
/// variant.
#[inline(always)]
fn dit_fft_g<const FAST: bool>(x: &mut [FixpDbl], ldn: i32) {
    if FAST {
        dit_fft(x, ldn, &SINE_TABLE_512, 512)
    } else if ARCH_ARM {
        crate::fft_rad2::dit_fft_arm(x, ldn, &SINE_TABLE_512, 512)
    } else {
        crate::fft_rad2::dit_fft_generic(x, ldn, &SINE_TABLE_512, 512)
    }
}

/// `ifft(length, pInput, scalefactor)`: no length is supported (`FDK_ASSERT(0)` in C); the data
/// are left unchanged.
pub fn ifft(length: i32, _p_input: &mut [FixpDbl], _scalefactor: &mut i32) {
    fdk_assert!(false, "IFFT length not supported: {length}");
}

/// Test inputs of the unit tests that compare the optimized kernels with the straight ports.
/// These run with every `FDK_AAC_RS_ARCH` variant on the host (the `cref` tests run the
/// variant of the C reference only).
#[cfg(test)]
pub(crate) mod testutil {
    /// xorshift64
    pub struct Rng(pub u64);

    impl Rng {
        pub fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        pub fn i32(&mut self) -> i32 {
            (self.next() >> 32) as i32
        }
        pub fn range(&mut self, lo: i32, hi: i32) -> i32 {
            lo + (self.next() % (hi - lo + 1) as u64) as i32
        }
    }

    /// Input vector `k` of `n` values: edge cases (zeros, impulses, full scale, alternating
    /// extremes), then random values with 0 to 31 bits of headroom, mostly 1 to 4.
    pub fn signal(r: &mut Rng, k: usize, n: usize) -> Vec<i32> {
        match k {
            0 => vec![0; n],
            1 => (0..n).map(|i| if i == 0 { i32::MAX } else { 0 }).collect(),
            2 => vec![i32::MAX; n],
            3 => vec![i32::MIN; n],
            4 => (0..n).map(|i| if i % 2 == 0 { i32::MAX } else { i32::MIN }).collect(),
            5 => (0..n).map(|_| if r.next() & 1 == 0 { i32::MIN } else { i32::MAX }).collect(),
            _ => {
                let shift = match r.next() % 8 {
                    0 => 0,
                    1 => r.range(5, 31) as u32,
                    _ => r.range(1, 4) as u32,
                };
                (0..n).map(|_| r.i32() >> shift).collect()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testutil::*;
    use super::*;

    /// Every length of `fft()`.
    pub const LENGTHS: &[i32] = &[
        2, 3, 4, 5, 6, 8, 10, 12, 15, 16, 20, 24, 32, 48, 60, 64, 80, 96, 120, 128, 192, 240, 256,
        384, 480, 512,
    ];

    #[test]
    fn fft_equals_straight_port() {
        let mut r = Rng(0x5eed_0001);
        for &len in LENGTHS {
            let n = 2 * len as usize;
            for k in 0..if n <= 64 { 4000 } else { 600 } {
                let x = signal(&mut r, k, n);
                let (mut a, mut b) = (x.clone(), x.clone());
                let (mut sa, mut sb) = (1, 1);
                fft(len, &mut a, &mut sa);
                fft_ref(len, &mut b, &mut sb);
                assert_eq!((a, sa), (b, sb), "fft({len}) case {k}");
            }
        }
        for k in 0..20000 {
            let x = signal(&mut r, k, 64);
            let (mut a, mut b) = (x.clone(), x.clone());
            fft_16(&mut a[..32]);
            fft_16_ref(&mut b[..32]);
            fft_32(&mut a);
            fft_32_ref(&mut b);
            assert_eq!(a, b, "fft_16/fft_32 case {k}");
        }
    }
}
