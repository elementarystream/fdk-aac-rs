// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Optimized loops of the inverse modulations of libFDK/src/qmf.cpp.

//! NEON blocks of four iterations of the loops after the DCT/DST of the inverse modulations.
//!
//! Every iteration reads and writes its own values (four values of the real and imaginary
//! parts, at `i`, `m - i`, `m + i`, `l - i` or `i`, `l - 1 - i`), and reads all of them before
//! it writes, so the iterations are independent; the blocks load the values of four iterations
//! (reversing the descending ones), compute lane-wise with the scalar operations (wrapping
//! negation, addition and subtraction, arithmetic shift) and store them.

use crate::dct::neon::rev;
use core::arch::aarch64::*;

/// The iterations `i = i0 .. i0 + 4` of the expansion loop of `qmfInverseModulationLP_even`
/// (`1 <= i0`, `i0 + 3 < m / 2`, `l = 2 m`): with the old real values `R`,
/// `im[m-i] = R[l-i]`, `im[m+i] = -R[l-i]`, `im[i] = R[m+i]`, `im[l-i] = -R[m+i]`,
/// `re[m+i] = R[i]`, `re[l-i] = R[m-i]`, `re[i] = R[m-i]`, `re[m-i] = R[i]`.
#[inline(always)]
pub(super) unsafe fn lp_even_expand_block(re: *mut i32, im: *mut i32, m: usize, i0: usize) {
    let l = 2 * m;
    let a = vld1q_s32(re.add(i0)); // R[i]
    let b = vld1q_s32(re.add(m - i0 - 3)); // R[m-i], reversed
    let c = vld1q_s32(re.add(m + i0)); // R[m+i]
    let d = vld1q_s32(re.add(l - i0 - 3)); // R[l-i], reversed
    vst1q_s32(im.add(m - i0 - 3), d);
    vst1q_s32(im.add(m + i0), vnegq_s32(rev(d)));
    vst1q_s32(im.add(i0), c);
    vst1q_s32(im.add(l - i0 - 3), vnegq_s32(rev(c)));
    vst1q_s32(re.add(m + i0), a);
    vst1q_s32(re.add(l - i0 - 3), b);
    vst1q_s32(re.add(i0), rev(b));
    vst1q_s32(re.add(m - i0 - 3), rev(a));
}

/// The iterations `i = i0 .. i0 + 4` of the final loop of `qmfInverseModulationHQ`
/// (`i0 + 3 < l / 2`): with `r1 = R[i]`, `i2 = I[l-1-i]`, `r2 = R[l-1-i]`, `i1 = I[i]`, negated
/// first if not `CLDFB`: `R[i] = (r1 - i1) >> 1`, `I[l-1-i] = -(r1 + i1) >> 1`,
/// `R[l-1-i] = (r2 - i2) >> 1`, `I[i] = -(r2 + i2) >> 1`.
#[inline(always)]
pub(super) unsafe fn hq_final_block<const CLDFB: bool>(
    re: *mut i32,
    im: *mut i32,
    l: usize,
    i0: usize,
) {
    let mut r1 = vld1q_s32(re.add(i0));
    let mut i1 = vld1q_s32(im.add(i0));
    let mut r2 = rev(vld1q_s32(re.add(l - 4 - i0)));
    let mut i2 = rev(vld1q_s32(im.add(l - 4 - i0)));
    if !CLDFB {
        r1 = vnegq_s32(r1);
        i1 = vnegq_s32(i1);
        r2 = vnegq_s32(r2);
        i2 = vnegq_s32(i2);
    }
    vst1q_s32(re.add(i0), vshrq_n_s32::<1>(vsubq_s32(r1, i1)));
    vst1q_s32(im.add(i0), vshrq_n_s32::<1>(vnegq_s32(vaddq_s32(r2, i2))));
    vst1q_s32(re.add(l - 4 - i0), rev(vshrq_n_s32::<1>(vsubq_s32(r2, i2))));
    vst1q_s32(im.add(l - 4 - i0), rev(vshrq_n_s32::<1>(vnegq_s32(vaddq_s32(r1, i1)))));
}

/// The iterations `i = i0 .. i0 + 4` of the final loop of `qmfInverseModulationLP_odd`
/// (`i0 + 3 < l / 2`): `p[i] = p[l-1-i]`, `p[2l-1-i] = -p[l+i]`.
#[inline(always)]
pub(super) unsafe fn lp_odd_final_block(p: *mut i32, l: usize, i0: usize) {
    let a = vld1q_s32(p.add(l - 4 - i0));
    let b = vld1q_s32(p.add(l + i0));
    vst1q_s32(p.add(i0), rev(a));
    vst1q_s32(p.add(2 * l - 4 - i0), rev(vnegq_s32(b)));
}
