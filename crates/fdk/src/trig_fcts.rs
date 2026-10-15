// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libFDK/include/FDK_trigFcts.h, libFDK/src/FDK_trigFcts.cpp.

//! Port of `FDK_trigFcts.h`: trigonometric functions, fixed point fractional implementation.
//!
//! Both variants compile the same code (`FUNCTION_inline_fixp_cos_sin` is defined by the header
//! itself, there is no `arm/` or `x86/` override); they differ only in the primitives (`fMult`,
//! `fDivNorm`, `fDivNormHighPrec`).
//!
//! `M_PI` is the double precision constant of `<math.h>` on both reference builds (the `float`
//! fallback of the header is not used).

use crate::common_fix::*;
use crate::fixpoint_math::{f_div_norm, f_div_norm_high_prec};
use crate::scale::scale_value;
use crate::tools_rom::SINE_TABLE_512;

/// `M_PI` of `<math.h>`.
const M_PI: f64 = core::f64::consts::PI;

/// `Q_ATANINP`: input in q25, output in q30.
pub const Q_ATANINP: i32 = 25;
/// `Q_ATANOUT`
pub const Q_ATANOUT: i32 = 30;
/// `ATI_SF`: 6
pub const ATI_SF: i32 = (DFRACT_BITS - 1) - Q_ATANINP;
/// `ATI_SCALE`
pub const ATI_SCALE: f32 = (1 << ATI_SF) as f32;
/// `ATO_SF`: 1 (] -pi/2 .. pi/2 [)
pub const ATO_SF: i32 = (DFRACT_BITS - 1) - Q_ATANOUT;
/// `ATO_SCALE`
pub const ATO_SCALE: f32 = (1 << ATO_SF) as f32;
/// `Q_ATAN2OUT`
pub const Q_ATAN2OUT: i32 = 29;
/// `AT2O_SF`: 2 (] -pi .. pi ])
pub const AT2O_SF: i32 = (DFRACT_BITS - 1) - Q_ATAN2OUT;
/// `AT2O_SCALE`
pub const AT2O_SCALE: f32 = (1 << AT2O_SF) as f32;

/// `Q(format)`: `(FIXP_DBL)(1 << format)`
#[inline(always)]
pub const fn q(format: i32) -> FixpDbl {
    1i32.wrapping_shl(format as u32)
}

/// `LD`: log2 of the length of the sine table [`SINE_TABLE_512`] (`SINETAB`).
pub const LD: i32 = 9;

/// `IMPROVE_ATAN2_ACCURACY`: 1 (65 dB SNR).
const MINSFTAB: i32 = 7;
const MAXSFTAB: i32 = 25;

/// `f_atan_expand_range[MAXSFTAB - (MINSFTAB - 1)]`: `fixp_atan()` output values outside of the
/// input range of `fixp_atan()`, to improve the SNR of `fixp_atan2()`.
///
/// `scripts/dump_table.py libFDK/src/FDK_trigFcts.cpp f_atan_expand_range LONG --hex --name F_ATAN_EXPAND_RANGE --per-line 4`
#[rustfmt::skip]
pub static F_ATAN_EXPAND_RANGE: [i32; 19] = [
    0x6387F2A6, 0x6407EDFC, 0x6447ED66, 0x6467ED54,
    0x6477ED51, 0x647FED51, 0x6483ED51, 0x6485ED51,
    0x6486ED51, 0x64876D51, 0x6487AD51, 0x6487CD51,
    0x6487DD51, 0x6487E551, 0x6487E951, 0x6487EB51,
    0x6487EC51, 0x6487ECD1, 0x6487ED11,
];

/// `fixp_atan2(y, x)`: `atan2(y, x)` in q29 (`AT2O_SF`).
///
/// Both builds compute the negations of `MINVAL_DBL` inputs with wrap-around (`neg`); the
/// division functions then get a negative operand (their `FDK_ASSERT`s are compiled out in the
/// reference).
pub fn fixp_atan2(y: FixpDbl, x: FixpDbl) -> FixpDbl {
    let q: FixpDbl;
    let at: FixpDbl; // atan out
    let mut sf: i32 = 0;

    // --- division
    if y > 0 {
        if x > 0 {
            q = f_div_norm_high_prec(y, x, &mut sf); // both pos.
        } else if x < 0 {
            q = f_div_norm_high_prec(y, x.wrapping_neg(), &mut sf).wrapping_neg();
        // x neg.
        } else {
            q = fl2fxconst_dbl(1.0); // y/x = pos/zero = +Inf
            sf = 0;
        }
    } else if y < 0 {
        if x > 0 {
            q = f_div_norm_high_prec(y.wrapping_neg(), x, &mut sf).wrapping_neg();
        // y neg.
        } else if x < 0 {
            q = f_div_norm_high_prec(y.wrapping_neg(), x.wrapping_neg(), &mut sf);
        // both neg.
        } else {
            q = fl2fxconst_dbl(-1.0); // y/x = neg/zero = -Inf
            sf = 0;
        }
    } else {
        q = 0;
        sf = 0;
    }
    let mut sfo = sf;

    // --- atan()
    if sfo > ATI_SF {
        // --- could not calc fixp_atan() here bec of input data out of range
        //     ==> therefore give back boundary values
        if sfo > MAXSFTAB {
            sfo = MAXSFTAB;
        }

        if q > 0 {
            at = F_ATAN_EXPAND_RANGE[(sfo - ATI_SF - 1) as usize];
        } else if q < 0 {
            at = F_ATAN_EXPAND_RANGE[(sfo - ATI_SF - 1) as usize].wrapping_neg();
        } else {
            // q == 0
            at = 0;
        }
    } else {
        // --- calc of fixp_atan() is possible; input data within range
        //     ==> set q on fixed scale level as desired from fixp_atan()
        let stf = sfo - ATI_SF;
        let q = if stf > 0 {
            q.wrapping_shl(f_min(stf, DFRACT_BITS - 1) as u32)
        } else {
            q >> f_min(-stf, DFRACT_BITS - 1)
        };
        at = fixp_atan(q); // ATO_SF
    }

    // --- atan2()
    let at2 = at >> (AT2O_SF - ATO_SF); // now AT2O_SF for atan2
    if x > 0 {
        at2
    } else if x < 0 {
        if y >= 0 {
            at2.wrapping_add(fl2fxconst_dbl(M_PI / AT2O_SCALE as f64))
        } else {
            at2.wrapping_sub(fl2fxconst_dbl(M_PI / AT2O_SCALE as f64))
        }
    } else {
        // x == 0
        if y > 0 {
            fl2fxconst_dbl(M_PI / 2.0 / AT2O_SCALE as f64)
        } else if y < 0 {
            fl2fxconst_dbl(-M_PI / 2.0 / AT2O_SCALE as f64)
        } else {
            0
        }
    }
}

/// `fixp_atan(x)`: `atan(x)` for `x` in q25, result in q30 (SNR 56 dB).
///
/// The C++ code negates negative inputs (`x = -x`) and then compares `x` with constants. Both
/// builds assume that the negation does not overflow and compare the absolute value unsigned
/// (`cneg` + `lsr #25`/`cmp`+`b.hi` on arm64, `neg`+`cmovs` + `cmp`+`ja` on x86): `MINVAL_DBL`
/// takes the branch for `|x| > 1.28` with `x == MINVAL_DBL`. The port does the same.
pub fn fixp_atan(x: FixpDbl) -> FixpDbl {
    // SNR of fixp_atan() = 56 dB
    const P281: FixpDbl = 0x00013000; // 0.281 in q18
    const ONEP571: FixpDbl = 0x6487ef00; // 1.571 in q30

    let sign = x < 0;
    let x = if sign { x.wrapping_neg() } else { x };

    fdk_assert!(fl2fxconst_dbl(1.0 / 64.0) == q(Q_ATANINP));
    let mut result: FixpDbl;
    // calc of arctan
    if (x as u32) < fl2fxconst_dbl(1.0 / 64.0) as u32 {
        // Chebyshev polynomial approximation of atan(x)
        // 7th-order approximation: atan(x) = a1*x + a2*x^3 + a3*x^5 + a3*x^7
        //   = x(a1 + x^2*(a2 + x^2*(a3 + a4*x^2)));
        // a1 = 0.9991334482227801, a2 = -0.3205332923816640, a3 = 0.1449824901444650,
        // a4 = -0.0382544649702990; 7th-order approximation in use (the most accurate solution)
        let x = x << ATI_SF;
        let x2 = f_pow2(x);
        let mut temp = f_mult_add_div2(
            fl2fxconst_dbl(0.1449824901444650f32 as f64) >> 1,
            x2,
            fl2fxconst_dbl(-0.0382544649702990),
        );
        temp = f_mult_add_div2(fl2fxconst_dbl(-0.3205332923816640f32 as f64) >> 2, x2, temp);
        temp = f_mult_add_div2(fl2fxconst_dbl(0.9991334482227801f32 as f64) >> 3, x2, temp);
        result = f_mult(x, temp.wrapping_shl(2));
    } else if (x as u32) < fl2fxconst_dbl(1.28 / 64.0) as u32 {
        let pi_by_4: FixpDbl = fl2fxconst_dbl(3.1415926 / 4.0) >> 1; // pi/4 in q30

        let delta_fix = x.wrapping_sub(fl2fxconst_dbl(1.0 / 64.0)) << 5; // q30
        result = pi_by_4.wrapping_add(delta_fix >> 1).wrapping_sub(f_pow2_div2(delta_fix));
    } else {
        // Other approximation for |x| > 1.28
        let mut res_e = 0;

        let mut temp = f_pow2_div2(x); // q25 * q25 - (DFRACT_BITS-1) - 1 = q18
        temp = temp.wrapping_add(P281); // q18 + q18 = q18
        result = f_div_norm(x, temp, &mut res_e);
        result =
            scale_value(result, (Q_ATANOUT - Q_ATANINP + 18 - DFRACT_BITS + 1).wrapping_add(res_e));
        result = ONEP571.wrapping_sub(result); // q30 + q30 = q30
    }
    if sign {
        result = result.wrapping_neg();
    }

    result
}

/// `fixp_sin_cos_residual_inline(x, scale, &sine, &cosine)`: calculates the coarse lookup index
/// and sign for sine and returns `(residual, sine, cosine)`, where `residual` is the delta x.
///
/// `scale` is the exponent of `x` (0..=21 keeps all shifts in range).
#[inline(always)]
pub fn fixp_sin_cos_residual_inline(x: FixpDbl, scale: i32) -> (FixpDbl, FixpDbl, FixpDbl) {
    let shift = 31 - scale - LD - 1;
    let mut ssign: i32 = 1;
    let mut csign: i32 = 1;

    let mut residual = f_mult(x, fl2fxconst_dbl(1.0 / M_PI));
    let mut s = residual.wrapping_shr(shift as u32);

    residual &= 1i32.wrapping_shl(shift as u32).wrapping_sub(1);
    residual = f_mult(residual, fl2fxconst_dbl(M_PI / 4.0)).wrapping_shl(2);
    residual = residual.wrapping_shl(scale as u32);

    // Sine sign symmetry
    if s & ((1 << LD) << 1) != 0 {
        ssign = -ssign;
    }
    // Cosine sign symmetry
    if s.wrapping_add(1 << LD) & ((1 << LD) << 1) != 0 {
        csign = -csign;
    }

    s = f_abs(s);

    s &= ((1 << LD) << 1) - 1; // Modulo PI

    if s > (1 << LD) {
        s = ((1 << LD) << 1) - s;
    }

    let (sl, cl): (i32, i32);
    // Because of packed table
    if s > (1 << (LD - 1)) {
        // Cosine/Sine symmetry for angles greater than PI/4
        s = (1 << LD) - s;
        let tmp = SINE_TABLE_512[s as usize];
        sl = tmp.re as i32;
        cl = tmp.im as i32;
    } else {
        let tmp = SINE_TABLE_512[s as usize];
        sl = tmp.im as i32;
        cl = tmp.re as i32;
    }

    let sine = (sl * ssign) << (DFRACT_BITS - FRACT_BITS);
    let cosine = (cl * csign) << (DFRACT_BITS - FRACT_BITS);

    (residual, sine, cosine)
}

/// `inline_fixp_cos_sin(x1, x2, scale, out)`: cosine and sine of two angles.
///
/// Returns `[cos(x1), sin(x1), cos(x2), sin(x2)]`; `scale` is the exponent of `x1` and `x2`.
#[inline(always)]
pub fn inline_fixp_cos_sin(x1: FixpDbl, x2: FixpDbl, scale: i32) -> [FixpDbl; 4] {
    let (residual, sine, cosine) = fixp_sin_cos_residual_inline(x1, scale);
    let error0 = f_mult_div2(sine, residual);
    let error1 = f_mult_div2(cosine, residual);
    let out0 = cosine.wrapping_sub(error0 << 1);
    let out1 = sine.wrapping_add(error1 << 1);

    let (residual, sine, cosine) = fixp_sin_cos_residual_inline(x2, scale);
    let error0 = f_mult_div2(sine, residual);
    let error1 = f_mult_div2(cosine, residual);
    let out2 = cosine.wrapping_sub(error0 << 1);
    let out3 = sine.wrapping_add(error1 << 1);

    [out0, out1, out2, out3]
}

/// `fixp_cos(x, scale)`
pub fn fixp_cos(x: FixpDbl, scale: i32) -> FixpDbl {
    let (residual, sine, cosine) = fixp_sin_cos_residual_inline(x, scale);
    let error = f_mult(sine, residual);
    cosine.wrapping_sub(error)
}

/// `fixp_sin(x, scale)`
pub fn fixp_sin(x: FixpDbl, scale: i32) -> FixpDbl {
    let (residual, sine, cosine) = fixp_sin_cos_residual_inline(x, scale);
    let error = f_mult(cosine, residual);
    sine.wrapping_add(error)
}

/// `fixp_cos_sin(x, scale, &cos, &sin)`: returns `(cos(x), sin(x))`.
pub fn fixp_cos_sin(x: FixpDbl, scale: i32) -> (FixpDbl, FixpDbl) {
    let (residual, sine, cosine) = fixp_sin_cos_residual_inline(x, scale);
    let error0 = f_mult(sine, residual);
    let error1 = f_mult(cosine, residual);
    (cosine.wrapping_sub(error0), sine.wrapping_add(error1))
}
