// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libFDK/include/fixpoint_math.h (optimized; no counterpart in the reference).

//! AArch64 NEON version of `CalcLdData()` (`fLog2(x, 0)`) for four values at a time, used by
//! [`super::ld_data_vector`].
//!
//! Every lane computes what [`super::f_log2_noexp`] computes for `x_e == 0`:
//!
//! - `x <= 0` gives `-1.0` (`MINVAL_DBL`), selected at the end;
//! - `b = clz(x) - 1` (0..=30), `xm = 2^31 - (x << b)` in `(0, 2^30]`;
//! - the polynomial as [`super::ld_poly`] computes it, every product with `sqdmulh` (the doubled
//!   high word `floor(2ab / 2^32)`), which cannot saturate because no operand is `MINVAL_DBL`:
//!   powers `floor(p * xm / 2^31)` (`aarch64`; for the other variants the halved powers `q`, with
//!   `floor(xm^2 / 2^32)` as the first one); terms `floor(c * p / 2^16)` as `sqdmulh(p, c << 15)`
//!   and `floor(c * q / 2^15)` as `sqdmulh(q, c << 16)`; the final `fMultAddDiv2(r, r, C)` as
//!   `r + (sqdmulh(r, C) >> 1)` (`floor(floor(2rC / 2^32) / 2) = floor(rC / 2^32)`);
//! - the exponent part of `fLog2()` and the scaling of `CalcLdData()`: for `b == 0`
//!   `r >> 5`; otherwise `e = 32 - fNorm(-b) = 33 - clz(b - 1)` (1..=6) and
//!   `((r >> (e - 1)) + (-b << (31 - e))) >> (6 - e)`, which for `e == 1` is the same formula.

use super::{LD_COEFF, LD_PRECISION};
use crate::{Arch, ARCH};
use core::arch::aarch64::*;

/// `CalcLdData()` of four values.
#[inline(always)]
pub(super) unsafe fn ld_data4(x: int32x4_t) -> int32x4_t {
    let one = vdupq_n_s32(1);
    let clz = vclzq_s32(x);
    let b = vsubq_s32(clz, one);
    let x2 = vshlq_s32(x, b);
    let xm = vsubq_s32(vdupq_n_s32(i32::MIN), x2);
    let mut r;
    if matches!(ARCH, Arch::Aarch64) {
        let mut p = xm;
        r = vqdmulhq_n_s32(p, (LD_COEFF[0] as i32) << 15);
        for &c in LD_COEFF[1..LD_PRECISION].iter() {
            p = vqdmulhq_s32(p, xm);
            r = vaddq_s32(r, vqdmulhq_n_s32(p, (c as i32) << 15));
        }
    } else {
        r = vqdmulhq_n_s32(xm, (LD_COEFF[0] as i32) << 15);
        let mut q = vshrq_n_s32::<1>(vqdmulhq_s32(xm, xm));
        r = vaddq_s32(r, vqdmulhq_n_s32(q, (LD_COEFF[1] as i32) << 16));
        for &c in LD_COEFF[2..LD_PRECISION].iter() {
            q = vqdmulhq_s32(q, xm);
            r = vaddq_s32(r, vqdmulhq_n_s32(q, (c as i32) << 16));
        }
    }
    // fMultAddDiv2(r, r, FL2FXCONST_DBL(2 * 0.4426950408889634))
    r = vaddq_s32(r, vshrq_n_s32::<1>(vqdmulhq_n_s32(r, super::LD_POLY_FINAL)));
    // e = 1 for b == 0, else 33 - clz(b - 1)
    let e = vsubq_s32(vdupq_n_s32(33), vclzq_s32(vsubq_s32(b, one)));
    let e = vbslq_s32(vceqzq_s32(b), one, e);
    let m = vaddq_s32(
        vshlq_s32(r, vsubq_s32(one, e)),
        vshlq_s32(vnegq_s32(b), vsubq_s32(vdupq_n_s32(31), e)),
    );
    let out = vshlq_s32(m, vsubq_s32(e, vdupq_n_s32(6)));
    vbslq_s32(vcgtzq_s32(x), out, vdupq_n_s32(i32::MIN))
}
