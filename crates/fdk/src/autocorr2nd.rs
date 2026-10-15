// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libFDK/include/autocorr2nd.h, libFDK/src/autocorr2nd.cpp.

//! Port of `autocorr2nd.h`: second order autocorrelation of real and complex QMF slots.
//!
//! Both variants compile the same code (`FUNCTION_autoCorr2nd_real` and `_cplx` are not
//! defined); the `.ii` files differ only in the shared headers. The functions read the two
//! values before `reBuffer` (`reBuffer[-2]`, `reBuffer[-1]`): the port takes slices that start
//! [`LPC_ORDER`] values before the C pointer, i.e. `re_buffer[LPC_ORDER + k]` is `reBuffer[k]`.
//! They hold `len + LPC_ORDER` values.
//!
//! The sums wrap around on overflow, and the normalization shifts by `mScale = -1` (when a sum
//! has its sign bit set) are taken modulo 32 like the `lsl`/`shl` instructions with a register
//! amount of both builds.

use crate::clz::fixnormz_d;
use crate::common_fix::*;

/// `ACORR_COEFS`
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(C)]
pub struct AcorrCoefs {
    pub r00r: FixpDbl,
    pub r11r: FixpDbl,
    pub r22r: FixpDbl,
    pub r01r: FixpDbl,
    pub r02r: FixpDbl,
    pub r12r: FixpDbl,
    pub r01i: FixpDbl,
    pub r02i: FixpDbl,
    pub r12i: FixpDbl,
    pub det: FixpDbl,
    pub det_scale: i32,
}

/// `LPC_ORDER`
pub const LPC_ORDER: usize = 2;

mod fast;

/// `autoCorr2nd_real(ac, reBuffer, len)`: calculates the autocorrelation of the real part.
/// `len >= 2` (number of QMF slots). Returns the scaling of the autocorrelation values.
///
/// The sums of the loop come from `fast::real_loop` (SIMD where available), which gives the
/// same values as the loop of the straight port [`auto_corr2nd_real_ref`].
pub fn auto_corr2nd_real(ac: &mut AcorrCoefs, re_buffer: &[FixpDbl], len: i32) -> i32 {
    let len_scale = f_max(32 - f_normz(len / 2), 1);
    let (accu1, accu3, accu5) = real_head(re_buffer, len_scale);
    let (accu1, accu3, accu5) = fast::real_loop(re_buffer, len, len_scale, accu1, accu3, accu5);
    real_tail(ac, re_buffer, len, len_scale, accu1, accu3, accu5)
}

/// The straight port of `autoCorr2nd_real()` (see [`auto_corr2nd_real`]).
pub fn auto_corr2nd_real_ref(ac: &mut AcorrCoefs, re_buffer: &[FixpDbl], len: i32) -> i32 {
    let len_scale = f_max(32 - f_normz(len / 2), 1);
    let (accu1, accu3, accu5) = real_head(re_buffer, len_scale);
    let (accu1, accu3, accu5) = real_loop_ref(re_buffer, len, len_scale, accu1, accu3, accu5);
    real_tail(ac, re_buffer, len, len_scale, accu1, accu3, accu5)
}

/// The sums of `autoCorr2nd_real()` before its loop: `(accu1, accu3, accu5)`.
#[inline(always)]
fn real_head(re_buffer: &[FixpDbl], len_scale: i32) -> (FixpDbl, FixpDbl, FixpDbl) {
    // pReBuf = realBuf - 2
    let mut p: usize = 0;
    let accu5 = f_mult_div2(re_buffer[p], re_buffer[p + 2])
        .wrapping_add(f_mult_div2(re_buffer[p + 1], re_buffer[p + 3]))
        >> len_scale;
    p += 1;
    let accu1 = f_pow2_div2(re_buffer[p]) >> len_scale;
    let accu3 = f_mult_div2(re_buffer[p], re_buffer[p + 1]) >> len_scale;
    (accu1, accu3, accu5)
}

/// The loop of `autoCorr2nd_real()`: adds its terms to `(accu1, accu3, accu5)`.
#[inline(always)]
fn real_loop_ref(
    re_buffer: &[FixpDbl],
    len: i32,
    len_scale: i32,
    mut accu1: FixpDbl,
    mut accu3: FixpDbl,
    mut accu5: FixpDbl,
) -> (FixpDbl, FixpDbl, FixpDbl) {
    let mut p: usize = 2;
    let mut j = (len - 2) >> 1;
    while j != 0 {
        accu1 = accu1.wrapping_add(
            f_pow2_div2(re_buffer[p]).wrapping_add(f_pow2_div2(re_buffer[p + 1])) >> len_scale,
        );
        accu3 = accu3.wrapping_add(
            f_mult_div2(re_buffer[p], re_buffer[p + 1])
                .wrapping_add(f_mult_div2(re_buffer[p + 1], re_buffer[p + 2]))
                >> len_scale,
        );
        accu5 = accu5.wrapping_add(
            f_mult_div2(re_buffer[p], re_buffer[p + 2])
                .wrapping_add(f_mult_div2(re_buffer[p + 1], re_buffer[p + 3]))
                >> len_scale,
        );
        j -= 1;
        p += 2;
    }
    (accu1, accu3, accu5)
}

/// The part of `autoCorr2nd_real()` after its loop.
#[inline(always)]
fn real_tail(
    ac: &mut AcorrCoefs,
    re_buffer: &[FixpDbl],
    len: i32,
    len_scale: i32,
    mut accu1: FixpDbl,
    mut accu3: FixpDbl,
    accu5: FixpDbl,
) -> i32 {
    // realBuf[k] is re_buffer[k + LPC_ORDER]
    let rb = |k: i32| re_buffer[(k + LPC_ORDER as i32) as usize];

    let mut accu2 = f_pow2_div2(rb(-2)) >> len_scale;
    accu2 = accu2.wrapping_add(accu1);

    accu1 = accu1.wrapping_add(f_pow2_div2(rb(len - 2)) >> len_scale);

    let mut accu4 = f_mult_div2(rb(-1), rb(-2)) >> len_scale;
    accu4 = accu4.wrapping_add(accu3);

    accu3 = accu3.wrapping_add(f_mult_div2(rb(len - 1), rb(len - 2)) >> len_scale);

    let m_scale = fixnormz_d(accu1 | accu2 | f_abs(accu3) | f_abs(accu4) | f_abs(accu5)) - 1;
    let auto_corr_scaling = m_scale - 1 - len_scale;

    // Scale to common scale factor
    ac.r11r = accu1.wrapping_shl(m_scale as u32);
    ac.r22r = accu2.wrapping_shl(m_scale as u32);
    ac.r01r = accu3.wrapping_shl(m_scale as u32);
    ac.r12r = accu4.wrapping_shl(m_scale as u32);
    ac.r02r = accu5.wrapping_shl(m_scale as u32);

    ac.det = f_mult_div2(ac.r11r, ac.r22r).wrapping_sub(f_mult_div2(ac.r12r, ac.r12r));
    // mScale = fixnorm_D(fAbs(ac->det)): both builds assume that fAbs() is not negative and
    // compute clz(|det|) - 1 (0 for det == 0), which gives -1 instead of 0 for det ==
    // MINVAL_DBL (fAbs(MINVAL_DBL) == MINVAL_DBL). det == MINVAL_DBL is not reachable: the
    // normalized r11r, r22r are either both in 0..2^31 or 0/MINVAL_DBL (mScale == -1).
    let m_scale = if ac.det == 0 { 0 } else { (f_abs(ac.det) as u32).leading_zeros() as i32 - 1 };

    ac.det = ac.det.wrapping_shl(m_scale as u32);
    ac.det_scale = m_scale - 1;

    auto_corr_scaling
}

/// `autoCorr2nd_cplx(ac, reBuffer, imBuffer, len)`: calculates the autocorrelation of the
/// complex values. `len >= 1` (number of QMF slots). Returns the scaling of the autocorrelation
/// values.
///
/// The sums of the loop come from `fast::cplx_loop` (SIMD where available), which gives the
/// same values as the loop of the straight port [`auto_corr2nd_cplx_ref`].
pub fn auto_corr2nd_cplx(
    ac: &mut AcorrCoefs,
    re_buffer: &[FixpDbl],
    im_buffer: &[FixpDbl],
    len: i32,
) -> i32 {
    let len_scale = f_max(32 - f_normz(len), 1);
    let sums = cplx_head(re_buffer, im_buffer, len_scale);
    let sums = fast::cplx_loop(re_buffer, im_buffer, len, len_scale, sums);
    cplx_tail(ac, re_buffer, im_buffer, len, len_scale, sums)
}

/// The straight port of `autoCorr2nd_cplx()` (see [`auto_corr2nd_cplx`]).
pub fn auto_corr2nd_cplx_ref(
    ac: &mut AcorrCoefs,
    re_buffer: &[FixpDbl],
    im_buffer: &[FixpDbl],
    len: i32,
) -> i32 {
    let len_scale = f_max(32 - f_normz(len), 1);
    let sums = cplx_head(re_buffer, im_buffer, len_scale);
    let sums = cplx_loop_ref(re_buffer, im_buffer, len, len_scale, sums);
    cplx_tail(ac, re_buffer, im_buffer, len, len_scale, sums)
}

/// The sums `[accu1, accu3, accu5, accu7, accu8]` of `autoCorr2nd_cplx()` before its loop.
#[inline(always)]
fn cplx_head(re_buffer: &[FixpDbl], im_buffer: &[FixpDbl], len_scale: i32) -> [FixpDbl; 5] {
    let accu1: FixpDbl = 0;
    let accu3: FixpDbl = 0;
    let accu5: FixpDbl = 0;
    let mut accu7: FixpDbl = 0;
    let mut accu8: FixpDbl = 0;

    // pReBuf = realBuf - 2, pImBuf = imagBuf - 2
    let p: usize = 0;
    accu7 = accu7.wrapping_add(
        f_mult_div2(re_buffer[p + 2], re_buffer[p])
            .wrapping_add(f_mult_div2(im_buffer[p + 2], im_buffer[p]))
            >> len_scale,
    );
    accu8 = accu8.wrapping_add(
        f_mult_div2(im_buffer[p + 2], re_buffer[p])
            .wrapping_sub(f_mult_div2(re_buffer[p + 2], im_buffer[p]))
            >> len_scale,
    );
    [accu1, accu3, accu5, accu7, accu8]
}

/// The loop of `autoCorr2nd_cplx()`: adds its terms to `[accu1, accu3, accu5, accu7, accu8]`.
#[inline(always)]
fn cplx_loop_ref(
    re_buffer: &[FixpDbl],
    im_buffer: &[FixpDbl],
    len: i32,
    len_scale: i32,
    sums: [FixpDbl; 5],
) -> [FixpDbl; 5] {
    let [mut accu1, mut accu3, mut accu5, mut accu7, mut accu8] = sums;
    // pReBuf = realBuf - 1, pImBuf = imagBuf - 1
    let mut p: usize = 1;
    let mut j = len - 1;
    while j != 0 {
        accu1 = accu1.wrapping_add(
            f_pow2_div2(re_buffer[p]).wrapping_add(f_pow2_div2(im_buffer[p])) >> len_scale,
        );
        accu3 = accu3.wrapping_add(
            f_mult_div2(re_buffer[p], re_buffer[p + 1])
                .wrapping_add(f_mult_div2(im_buffer[p], im_buffer[p + 1]))
                >> len_scale,
        );
        accu5 = accu5.wrapping_add(
            f_mult_div2(im_buffer[p + 1], re_buffer[p])
                .wrapping_sub(f_mult_div2(re_buffer[p + 1], im_buffer[p]))
                >> len_scale,
        );
        accu7 = accu7.wrapping_add(
            f_mult_div2(re_buffer[p + 2], re_buffer[p])
                .wrapping_add(f_mult_div2(im_buffer[p + 2], im_buffer[p]))
                >> len_scale,
        );
        accu8 = accu8.wrapping_add(
            f_mult_div2(im_buffer[p + 2], re_buffer[p])
                .wrapping_sub(f_mult_div2(re_buffer[p + 2], im_buffer[p]))
                >> len_scale,
        );
        j -= 1;
        p += 1;
    }
    [accu1, accu3, accu5, accu7, accu8]
}

/// The part of `autoCorr2nd_cplx()` after its loop.
#[inline(always)]
fn cplx_tail(
    ac: &mut AcorrCoefs,
    re_buffer: &[FixpDbl],
    im_buffer: &[FixpDbl],
    len: i32,
    len_scale: i32,
    sums: [FixpDbl; 5],
) -> i32 {
    // realBuf[k] is re_buffer[k + LPC_ORDER], imagBuf[k] is im_buffer[k + LPC_ORDER]
    let rb = |k: i32| re_buffer[(k + LPC_ORDER as i32) as usize];
    let ib = |k: i32| im_buffer[(k + LPC_ORDER as i32) as usize];
    let [mut accu1, mut accu3, mut accu5, accu7, accu8] = sums;

    let mut accu2 = f_pow2_div2(rb(-2)).wrapping_add(f_pow2_div2(ib(-2))) >> len_scale;
    accu2 = accu2.wrapping_add(accu1);

    accu1 = accu1
        .wrapping_add(f_pow2_div2(rb(len - 2)).wrapping_add(f_pow2_div2(ib(len - 2))) >> len_scale);
    let mut accu0 = (f_pow2_div2(rb(len - 1)).wrapping_add(f_pow2_div2(ib(len - 1))) >> len_scale)
        .wrapping_sub(f_pow2_div2(rb(-1)).wrapping_add(f_pow2_div2(ib(-1))) >> len_scale);
    accu0 = accu0.wrapping_add(accu1);

    let mut accu4 =
        f_mult_div2(rb(-1), rb(-2)).wrapping_add(f_mult_div2(ib(-1), ib(-2))) >> len_scale;
    accu4 = accu4.wrapping_add(accu3);

    accu3 = accu3.wrapping_add(
        f_mult_div2(rb(len - 1), rb(len - 2)).wrapping_add(f_mult_div2(ib(len - 1), ib(len - 2)))
            >> len_scale,
    );

    let mut accu6 =
        f_mult_div2(ib(-1), rb(-2)).wrapping_sub(f_mult_div2(rb(-1), ib(-2))) >> len_scale;
    accu6 = accu6.wrapping_add(accu5);

    accu5 = accu5.wrapping_add(
        f_mult_div2(ib(len - 1), rb(len - 2)).wrapping_sub(f_mult_div2(rb(len - 1), ib(len - 2)))
            >> len_scale,
    );

    let m_scale = fixnormz_d(
        accu0
            | accu1
            | accu2
            | f_abs(accu3)
            | f_abs(accu4)
            | f_abs(accu5)
            | f_abs(accu6)
            | f_abs(accu7)
            | f_abs(accu8),
    ) - 1;
    let auto_corr_scaling = m_scale - 1 - len_scale;

    // Scale to common scale factor
    let sh = m_scale as u32;
    ac.r00r = accu0.wrapping_shl(sh);
    ac.r11r = accu1.wrapping_shl(sh);
    ac.r22r = accu2.wrapping_shl(sh);
    ac.r01r = accu3.wrapping_shl(sh);
    ac.r12r = accu4.wrapping_shl(sh);
    ac.r01i = accu5.wrapping_shl(sh);
    ac.r12i = accu6.wrapping_shl(sh);
    ac.r02r = accu7.wrapping_shl(sh);
    ac.r02i = accu8.wrapping_shl(sh);

    ac.det = (f_mult_div2(ac.r11r, ac.r22r) >> 1).wrapping_sub(
        f_mult_div2(ac.r12r, ac.r12r).wrapping_add(f_mult_div2(ac.r12i, ac.r12i)) >> 1,
    );
    let m_scale = fixnormz_d(f_abs(ac.det)) - 1;

    ac.det = ac.det.wrapping_shl(m_scale as u32);
    ac.det_scale = m_scale - 2;

    auto_corr_scaling
}
