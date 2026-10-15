// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libFDK/include/x86/fixpoint_math_x86.h (as compiled for 32-bit x86).

//! Software model of the x87 floating point arithmetic of the 32-bit x86 build (`ref-i686`, the
//! `x86_32` variant; PORTING.md, "Floating point and the x87 variant").
//!
//! clang's default 32-bit x86 target (`-m32`: i386-pc-linux-gnu, CPU i686) has no SSE, so the
//! reference computes `float` and `double` expressions with the x87 unit: the operations round to
//! the 64-bit significand of the extended precision format (precision control "extended", the
//! Linux default control word 0x037f; round to nearest even), and values are only rounded to
//! `float` or `double` where the compiled code stores them to memory, e.g. for explicit
//! conversions to `float` that are passed to `frexpf` (`fstps`). Conversions from `int` are exact
//! (`fild`) and are not rounded to the type of the C expression: `(float)(INT)op` is not rounded
//! to 24 bits. Products of an `int` and a `float` constant are exact (at most 55 significant
//! bits).
//!
//! [`F80`] reproduces the operations used by the reference (multiplication, addition, division,
//! square root, comparison, rounding to `float`, truncation to `int` like `fistp` with the
//! rounding mode "toward zero") for finite values and NaN. The values in the library never get
//! near the limits of the exponent range (subnormal `float` results are not modelled). Infinities
//! cannot occur: the library divides only by non-zero values.
//!
//! The x87 expressions of the library ([`sqrt_fixp`], [`inv_sqrt_norm2_f32`], [`inv_fixp_f32`],
//! [`fl2fxconst_dbl_mul2`]) are computed by the implementation selected at build time
//! (`FDK_AAC_RS_X87`, see build.rs): `f64` arithmetic with a proven error bound ([`fast`]; the
//! default wherever `f64` is IEEE double precision), the x87 instructions of the reference
//! (`asm`; x86 and x86-64 targets, the default on 32-bit x86 without SSE2), or the model
//! ([`soft`]). All three give the same results.

/// A value of the x87 extended precision format: `(-1)^neg * mant * 2^(exp - 63)` with a
/// normalized significand (`mant >= 2^63`), or zero (`mant == 0`), or NaN.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct F80 {
    neg: bool,
    exp: i32,
    mant: u64,
    nan: bool,
}

impl F80 {
    pub const ZERO: F80 = F80 { neg: false, exp: 0, mant: 0, nan: false };
    pub const ONE: F80 = F80 { neg: false, exp: 0, mant: 1 << 63, nan: false };
    pub const NAN: F80 = F80 { neg: false, exp: 0, mant: 0, nan: true };

    /// `true` for NaN.
    pub fn is_nan(self) -> bool {
        self.nan
    }

    /// The value `wide * 2^wide_exp` (plus less than one unit of `wide` if `sticky`), rounded to
    /// a 64 bit significand (round to nearest, ties to even).
    fn round(neg: bool, wide: u128, wide_exp: i32, sticky: bool) -> F80 {
        if wide == 0 {
            return F80 { neg, ..F80::ZERO };
        }
        let bits = 128 - wide.leading_zeros() as i32;
        let exp = wide_exp + bits - 1;
        if bits <= 64 {
            // exact, or the sticky rest is below half a unit
            return F80 { neg, exp, mant: (wide << (64 - bits)) as u64, nan: false };
        }
        let shift = (bits - 64) as u32;
        let mut mant = (wide >> shift) as u64;
        let rest = wide & ((1u128 << shift) - 1);
        let half = 1u128 << (shift - 1);
        let up = rest > half || (rest == half && (sticky || mant & 1 != 0));
        let mut exp = exp;
        if up {
            mant = mant.wrapping_add(1);
            if mant == 0 {
                mant = 1 << 63;
                exp += 1;
            }
        }
        F80 { neg, exp, mant, nan: false }
    }

    /// `fild`: exact conversion of an integer.
    pub fn from_i32(v: i32) -> F80 {
        F80::round(v < 0, v.unsigned_abs() as u128, 0, false)
    }

    /// `fild` of a 64 bit integer (exact, the significand has 64 bits).
    pub fn from_i64(v: i64) -> F80 {
        F80::round(v < 0, v.unsigned_abs() as u128, 0, false)
    }

    /// `fld` of a `float` (exact).
    pub fn from_f32(v: f32) -> F80 {
        F80::from_f64(v as f64)
    }

    /// `fld` of a `double` (exact).
    pub fn from_f64(v: f64) -> F80 {
        if v.is_nan() {
            return F80::NAN;
        }
        let bits = v.to_bits();
        let neg = bits >> 63 != 0;
        let biased = ((bits >> 52) & 0x7ff) as i32;
        let frac = bits & ((1u64 << 52) - 1);
        if biased == 0x7ff {
            // Infinities are not modelled (they never occur in the library).
            return F80::NAN;
        }
        if biased == 0 {
            // zero or subnormal
            return F80::round(neg, frac as u128, -1074, false);
        }
        F80::round(neg, (frac | (1u64 << 52)) as u128, biased - 1075, false)
    }

    /// `fmul`
    #[allow(clippy::should_implement_trait)] // named after the x87 instruction it models
    pub fn mul(self, b: F80) -> F80 {
        if self.nan || b.nan {
            return F80::NAN;
        }
        let neg = self.neg != b.neg;
        F80::round(neg, self.mant as u128 * b.mant as u128, self.exp + b.exp - 126, false)
    }

    /// `fadd`
    #[allow(clippy::should_implement_trait)] // named after the x87 instruction it models
    pub fn add(self, b: F80) -> F80 {
        if self.nan || b.nan {
            return F80::NAN;
        }
        if self.mant == 0 {
            return if b.mant == 0 { F80 { neg: self.neg && b.neg, ..F80::ZERO } } else { b };
        }
        if b.mant == 0 {
            return self;
        }
        let (big, small) =
            if (self.exp, self.mant) >= (b.exp, b.mant) { (self, b) } else { (b, self) };
        // 62 guard bits below the significand of the larger operand
        let a = (big.mant as u128) << 62;
        let d = (big.exp - small.exp) as u32;
        let full = (small.mant as u128) << 62;
        let (c, sticky) =
            if d >= 127 { (0, true) } else { (full >> d, full & ((1u128 << d) - 1) != 0) };
        let wide_exp = big.exp - 63 - 62;
        if big.neg == small.neg {
            F80::round(big.neg, a + c, wide_exp, sticky)
        } else if a == c && !sticky {
            F80::ZERO
        } else {
            // the sticky rest of the subtrahend lowers the difference below a - c
            F80::round(big.neg, a - c - sticky as u128, wide_exp, sticky)
        }
    }

    /// `fucomi`: the order of two values; `None` if one of them is NaN (unordered).
    pub fn partial_cmp(self, b: F80) -> Option<core::cmp::Ordering> {
        use core::cmp::Ordering;
        if self.nan || b.nan {
            return None;
        }
        // Zeros of either sign are equal.
        let key = |v: F80| -> (i32, i32, u64) {
            if v.mant == 0 {
                (0, 0, 0)
            } else if v.neg {
                (-1, -v.exp, !v.mant)
            } else {
                (1, v.exp, v.mant)
            }
        };
        let (a, b) = (key(self), key(b));
        Some(if a.0 != b.0 {
            a.0.cmp(&b.0)
        } else if a.0 == 0 {
            Ordering::Equal
        } else {
            (a.1, a.2).cmp(&(b.1, b.2))
        })
    }

    /// `fsub`
    #[allow(clippy::should_implement_trait)] // named after the x87 instruction it models
    pub fn sub(self, b: F80) -> F80 {
        self.add(F80 { neg: !b.neg, ..b })
    }

    /// `fdiv`: `self / b` (`b` must not be zero).
    #[allow(clippy::should_implement_trait)] // named after the x87 instruction it models
    pub fn div(self, b: F80) -> F80 {
        if self.nan || b.nan {
            return F80::NAN;
        }
        if b.mant == 0 {
            // Division by zero is not modelled (the library never divides by zero here).
            return F80::NAN;
        }
        let neg = self.neg != b.neg;
        if self.mant == 0 {
            return F80 { neg, ..F80::ZERO };
        }
        let d = b.mant as u128;
        let num = (self.mant as u128) << 64;
        let (q, r) = (num / d, num % d);
        // 62 more quotient bits and the sticky bit of the rest
        let r2 = r << 62;
        let wide = (q << 62) | (r2 / d);
        F80::round(neg, wide, self.exp - b.exp - 126, r2 % d != 0)
    }

    /// `fsqrt` (NaN for negative values).
    pub fn sqrt(self) -> F80 {
        if self.nan || (self.neg && self.mant != 0) {
            return F80::NAN;
        }
        if self.mant == 0 {
            return self;
        }
        // value = mant * 2^e with e = exp - 63; n = mant * 2^k in [2^126, 2^128) with e - k even
        let e = self.exp - 63;
        let k = if (e - 63) % 2 == 0 { 63 } else { 64 };
        let n = (self.mant as u128) << k;
        let s = n.isqrt();
        let rest = n - s * s;
        // s has 64 bits; the exact root lies in [s, s + 1) and is never s + 1/2
        let half_exp = (e - k) / 2;
        F80::round(false, (s << 1) | (rest > s) as u128, half_exp - 1, rest != 0)
    }

    /// Rounding to `float` (`fstps`), round to nearest even.
    pub fn to_f32(self) -> f32 {
        if self.nan {
            return f32::NAN;
        }
        self.to_bits(24) as f32
    }

    /// Rounding to `double` (`fstpl`), round to nearest even.
    pub fn to_f64(self) -> f64 {
        if self.nan {
            return f64::NAN;
        }
        self.to_bits(53)
    }

    /// The value rounded to a significand of `p <= 53` bits, as an `f64` (exact).
    fn to_bits(self, p: u32) -> f64 {
        if self.mant == 0 {
            return if self.neg { -0.0 } else { 0.0 };
        }
        let shift = 64 - p;
        let mut m = self.mant >> shift;
        let rest = self.mant & ((1u64 << shift) - 1);
        let half = 1u64 << (shift - 1);
        let mut exp = self.exp;
        if rest > half || (rest == half && m & 1 != 0) {
            m += 1;
            if m == 1 << p {
                m >>= 1;
                exp += 1;
            }
        }
        // m * 2^(exp - p + 1), exact in f64 for the exponent range used here
        let v = m as f64 * pow2(exp - p as i32 + 1);
        if self.neg {
            -v
        } else {
            v
        }
    }

    /// `fistp` with the rounding mode "toward zero" (as the compiled C conversion to `int`):
    /// `INT_MIN` ("integer indefinite") for NaN and values out of range.
    pub fn to_i32_trunc(self) -> i32 {
        match self.to_i64_trunc_checked(31) {
            Some(v) => v as i32,
            None => i32::MIN,
        }
    }

    /// `fistpll` with the rounding mode "toward zero": `INT64_MIN` for NaN and values out of
    /// range.
    pub fn to_i64_trunc(self) -> i64 {
        self.to_i64_trunc_checked(63).unwrap_or(i64::MIN)
    }

    /// The conversion to `uint64_t` of the compiled C code (`fistpll` of the value minus 2^63
    /// for values from 2^63, the sign bit of the result toggled): truncation toward zero for
    /// values in `0 .. 2^64`.
    pub fn to_u64_trunc(self) -> u64 {
        let two63 = F80 { neg: false, exp: 63, mant: 1 << 63, nan: false };
        if !self.nan && !self.neg && self.exp >= 63 {
            (self.sub(two63).to_i64_trunc() as u64) ^ (1 << 63)
        } else {
            self.to_i64_trunc() as u64
        }
    }

    /// The value truncated toward zero if it fits into a signed integer of `bits + 1` bits.
    fn to_i64_trunc_checked(self, bits: i32) -> Option<i64> {
        if self.nan {
            return None;
        }
        if self.mant == 0 || self.exp < 0 {
            return Some(0);
        }
        if self.exp > bits {
            return None;
        }
        let mag = (self.mant >> (63 - self.exp)) as i128;
        let v = if self.neg { -mag } else { mag };
        let lim = 1i128 << bits;
        if v >= lim || v < -lim {
            None
        } else {
            Some(v as i64)
        }
    }
}

/// `2^e` as `f64` (normal range).
fn pow2(e: i32) -> f64 {
    let mut v = 1.0f64;
    let mut e = e;
    while e > 1000 {
        v *= f64::from_bits(((1023 + 1000) as u64) << 52);
        e -= 1000;
    }
    while e < -1000 {
        v *= f64::from_bits(((1023 - 1000) as u64) << 52);
        e += 1000;
    }
    v * f64::from_bits(((1023 + e) as u64) << 52)
}

// ---------------------------------------------------------------------------------------------
// The x87 expressions of the library
// ---------------------------------------------------------------------------------------------
//
// Each expression has three implementations with the same results (selected at build time by
// `FDK_AAC_RS_X87`, see build.rs; the selected one is re-exported here):
//
// - [`soft`]: the software model [`F80`] (every host).
// - [`hw`]: the x87 instructions of the reference (x86 and x86-64 hosts), exact by construction.
// - [`fast`]: `f64` arithmetic with a proven error bound where it decides the result, [`soft`]
//   where it does not (every host whose `f64` arithmetic is IEEE double precision).

/// `46340.9492f` of `sqrtFixp` (`x86/fixpoint_math_x86.h`).
const SQRT_FIXP_SCALE: f32 = 46340.9492;

#[cfg(fdk_x87 = "asm")]
pub use hw::{fl2fxconst_dbl_mul2, inv_fixp_f32, inv_sqrt_norm2_f32, sqrt_fixp};

#[cfg(fdk_x87 = "f64")]
pub use fast::{fl2fxconst_dbl_mul2, inv_fixp_f32, inv_sqrt_norm2_f32, sqrt_fixp};

#[cfg(fdk_x87 = "soft")]
pub use soft::{fl2fxconst_dbl_mul2, inv_fixp_f32, inv_sqrt_norm2_f32, sqrt_fixp};

/// The x87 expressions computed with the software model [`F80`].
pub mod soft {
    use super::{F80, SQRT_FIXP_SCALE};
    use core::cmp::Ordering::{Equal, Greater};

    /// `(INT)(sqrt((float)(INT)op) * 46340.9492f)` of `sqrtFixp`: `fildl`, `fsqrt`, `fmuls`,
    /// `fistpl` with the rounding mode "toward zero". Negative `op` gives `INT_MIN` (NaN).
    pub fn sqrt_fixp(op: i32) -> i32 {
        F80::from_i32(op).sqrt().mul(F80::from_f32(SQRT_FIXP_SCALE)).to_i32_trunc()
    }

    /// `(float)(1.0 / sqrt(0.5f * (float)op))` of `invSqrtNorm2` (`op != 0`): `fildl`,
    /// `fmuls 0.5f`, `fsqrt`, `fld1`, `fdivp`, `fstps`. Negative `op` gives NaN.
    pub fn inv_sqrt_norm2_f32(op: i32) -> f32 {
        F80::ONE.div(F80::from_i32(op).mul(F80::from_f32(0.5)).sqrt()).to_f32()
    }

    /// `(float)(1.0 / (float)op)` of both `invFixp` (`op != 0`): `fld1`, `fidivl`, `fstps`.
    pub fn inv_fixp_f32(op: i32) -> f32 {
        F80::ONE.div(F80::from_i32(op)).to_f32()
    }

    /// `FL2FXCONST_DBL((float)i * c1 * c2)` evaluated at run time: `fildl`, `fmuls c1`,
    /// `fmuls c2`, the comparison with 0, `fmuls 2^31`, `fadds 0.5f` resp. `fadds -0.5f`, the
    /// comparison with `2147483647.0` resp. `-2147483648.0f` (saturation), `fistpl`.
    pub fn fl2fxconst_dbl_mul2(i: i32, c1: f32, c2: f32) -> i32 {
        let val = F80::from_i32(i).mul(F80::from_f32(c1)).mul(F80::from_f32(c2));
        let scaled = val.mul(F80::from_f32(2147483648.0f32));
        if matches!(val.partial_cmp(F80::ZERO), Some(Greater | Equal)) {
            let v = scaled.add(F80::from_f32(0.5));
            if matches!(v.partial_cmp(F80::from_f64(2147483647.0)), Some(Greater | Equal)) {
                i32::MAX
            } else {
                v.to_i32_trunc()
            }
        } else {
            let v = scaled.sub(F80::from_f32(0.5));
            if matches!(F80::from_f32(-2147483648.0f32).partial_cmp(v), Some(Greater | Equal)) {
                i32::MIN
            } else {
                v.to_i32_trunc()
            }
        }
    }

    /// `(INT)((float)op * k)`: `fildl`, `fmuls k`, `fistpl` with the rounding mode "toward
    /// zero" (`INT_MIN` out of range).
    pub fn mul_trunc(op: i32, k: f32) -> i32 {
        F80::from_i32(op).mul(F80::from_f32(k)).to_i32_trunc()
    }

    /// `(float)a < hi * (float)l && (float)a > lo * (float)l`: `fildl`, `fmuls`, `fucomip`.
    pub fn between_products(a: i32, l: i32, lo: f32, hi: f32) -> bool {
        use core::cmp::Ordering::Less;
        let (a, l) = (F80::from_i32(a), F80::from_i32(l));
        a.partial_cmp(l.mul(F80::from_f32(hi))) == Some(Less)
            && a.partial_cmp(l.mul(F80::from_f32(lo))) == Some(Greater)
    }
}

/// The x87 expressions of `x86/fixpoint_math_x86.h` computed in `f64`, exact where a proven
/// error bound decides the result, and with [`soft`] elsewhere.
///
/// Exactness argument (`u = 2^-53`, `f64` operations correctly rounded, x87 operations correctly
/// rounded to 64 bits, relative error at most `2^-64`):
///
/// - `sqrtFixp`: for `0 <= op < 2^31` the exact value is `t = sqrt(op) * c < 2^31` (`c` =
///   `46340.9492f`). The x87 result `r` (two roundings to 64 bits) satisfies
///   `|r - t| <= t * (2^-63 + 2^-128) < 2^-31`, the `f64` result `y` (two roundings to 53 bits)
///   `|y - t| <= t * (2^-52 + 2^-106) < 2^-20.9`, so `|y - r| < 2^-20`. If the fraction of `y`
///   lies in `(2^-20, 1 - 2^-20)`, `r` lies strictly between the same two integers as `y` and
///   `fistp` (toward zero, `r >= 0`) gives `trunc(y)`. Otherwise (about 2^-19 of the inputs,
///   and exact products such as `op = 65536`) the model decides.
/// - `invSqrtNorm2`, `invFixp`: the result is `RN24(q)` of the x87 quotient `q`. The `f64`
///   quotient `p` differs from the exact value by less than 2.02 units in the last place of
///   `p` (one rounding for `1 / op`, two for `1 / sqrt(op / 2)`; `op / 2` is exact), `q` by
///   less than `2^-10` of them. `RN24` is constant between consecutive midpoints of `float`
///   values. Within the binade of `p` the midpoints are the `f64` values whose 29 low
///   significand bits are `2^28`; across a binade boundary the nearest midpoint is `2^27` units
///   away. So if the low bits of `p` differ from `2^28` by more than 8, no midpoint lies
///   between `p` and `q`, and `RN24(q) = RN24(p)` (`p as f32`). The values are normal in both
///   formats (`2^-31 <= |p| <= 2^0.5`).
pub mod fast {
    pub use super::soft::fl2fxconst_dbl_mul2;
    use super::{soft, SQRT_FIXP_SCALE};

    /// The fraction band where `sqrt_fixp` falls back to the model.
    const SQRT_FIXP_TOL: f64 = 1.0 / (1u32 << 20) as f64;
    /// The distance (in units in the last place) from a `float` midpoint where the quotients
    /// fall back to the model.
    const MIDPOINT_MARGIN: u64 = 8;

    /// [`soft::sqrt_fixp`] if the `f64` bound decides it, `None` otherwise.
    #[inline]
    pub fn try_sqrt_fixp(op: i32) -> Option<i32> {
        if op <= 0 {
            // 0 is exact; NaN gives the "integer indefinite" INT_MIN
            return Some(if op == 0 { 0 } else { i32::MIN });
        }
        let y = (op as f64).sqrt() * SQRT_FIXP_SCALE as f64;
        let t = y as i32; // y < 2^31
        let frac = y - t as f64; // exact
        if frac > SQRT_FIXP_TOL && frac < 1.0 - SQRT_FIXP_TOL {
            Some(t)
        } else {
            None
        }
    }

    /// `RN24(p)` if no `float` midpoint is within [`MIDPOINT_MARGIN`] units of `p`.
    #[inline(always)]
    fn round_f32(p: f64) -> Option<f32> {
        let low = p.to_bits() & ((1 << 29) - 1);
        if low.abs_diff(1 << 28) > MIDPOINT_MARGIN {
            Some(p as f32)
        } else {
            None
        }
    }

    /// [`soft::inv_sqrt_norm2_f32`] for `op > 0` if the `f64` bound decides it, `None`
    /// otherwise.
    #[inline]
    pub fn try_inv_sqrt_norm2_f32(op: i32) -> Option<f32> {
        if op <= 0 {
            return None;
        }
        round_f32(1.0 / (op as f64 * 0.5).sqrt())
    }

    /// [`soft::inv_fixp_f32`] for `op != 0` if the `f64` bound decides it, `None` otherwise.
    #[inline]
    pub fn try_inv_fixp_f32(op: i32) -> Option<f32> {
        if op == 0 {
            return None;
        }
        round_f32(1.0 / op as f64)
    }

    #[cold]
    #[inline(never)]
    fn cold<T>(op: i32, f: fn(i32) -> T) -> T {
        f(op)
    }

    /// [`soft::sqrt_fixp`]
    #[inline]
    pub fn sqrt_fixp(op: i32) -> i32 {
        try_sqrt_fixp(op).unwrap_or_else(|| cold(op, soft::sqrt_fixp))
    }

    /// [`soft::inv_sqrt_norm2_f32`]
    #[inline]
    pub fn inv_sqrt_norm2_f32(op: i32) -> f32 {
        try_inv_sqrt_norm2_f32(op).unwrap_or_else(|| cold(op, soft::inv_sqrt_norm2_f32))
    }

    /// [`soft::inv_fixp_f32`]
    #[inline]
    pub fn inv_fixp_f32(op: i32) -> f32 {
        try_inv_fixp_f32(op).unwrap_or_else(|| cold(op, soft::inv_fixp_f32))
    }
}

/// The x87 expressions computed with the x87 instructions of the reference (`ref-i686`, clang 19
/// `-m32 -O3`; addresses in the object files of `work/fdk-aac-asm/ref-i686`).
///
/// Each block saves the caller's control word (`fnstcw`), computes with the control word of the
/// reference's environment, [`CW_NEAREST`](hw::CW_NEAREST) (0x037f, the Linux default:
/// exceptions masked, extended precision, round to nearest even), truncates with
/// [`CW_CHOP`](hw::CW_CHOP) (0x0f7f, the reference's `or $0xc00` of that control word) and
/// restores the caller's control word
/// (`fldcw`). The reference computes with the control word it finds; loading 0x037f explicitly
/// makes the results independent of the caller (e.g. a host that sets double precision).
///
/// The asm blocks clobber all x87 registers, so the x87 stack is empty on entry, and leave it
/// empty.
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
pub mod hw {
    use super::SQRT_FIXP_SCALE;
    use core::arch::asm;
    use core::mem::offset_of;

    /// The control word of the reference's environment.
    pub const CW_NEAREST: u16 = 0x037f;
    /// [`CW_NEAREST`] with the rounding mode "toward zero".
    pub const CW_CHOP: u16 = 0x0f7f;

    /// The memory operands of an asm block.
    #[repr(C)]
    struct Frame {
        /// `double` constant (`{max}`)
        max: f64,
        /// `int` operands (`{a}`, `{b}`)
        a: i32,
        b: i32,
        /// `float` constants (`{k0}` .. `{k5}`)
        k: [f32; 6],
        /// results (`{out}`, `{which}`)
        out: i32,
        which: i32,
        /// control words (`{cw_saved}`, `{cw_nearest}`, `{cw_chop}`)
        cw_saved: u16,
        cw_nearest: u16,
        cw_chop: u16,
    }

    impl Frame {
        #[inline(always)]
        fn new(a: i32, b: i32, k: [f32; 6]) -> Frame {
            Frame {
                max: 0.0,
                a,
                b,
                k,
                out: 0,
                which: 0,
                cw_saved: 0,
                cw_nearest: CW_NEAREST,
                cw_chop: CW_CHOP,
            }
        }
    }

    /// `asm!` with the memory operands of [`Frame`] (`{a}`, `{out}`, ...) addressed through
    /// `{f}`, all x87 registers clobbered.
    macro_rules! x87 {
        ($frame:expr, $($t:tt)*) => {{
            let frame: *mut Frame = $frame;
            // SAFETY: the block reads and writes only the frame and the x87 registers, which
            // are all declared as clobbered; it leaves the x87 stack empty and restores the
            // control word.
            unsafe {
                asm!(
                    $($t)*
                    "/* {a} {b} {k0} {k1} {k2} {k3} {k4} {k5} {out} {which} {max} */",
                    "/* {cw_saved} {cw_nearest} {cw_chop} */",
                    f = in(reg) frame,
                    a = const offset_of!(Frame, a),
                    b = const offset_of!(Frame, b),
                    k0 = const offset_of!(Frame, k),
                    k1 = const offset_of!(Frame, k) + 4,
                    k2 = const offset_of!(Frame, k) + 8,
                    k3 = const offset_of!(Frame, k) + 12,
                    k4 = const offset_of!(Frame, k) + 16,
                    k5 = const offset_of!(Frame, k) + 20,
                    out = const offset_of!(Frame, out),
                    which = const offset_of!(Frame, which),
                    cw_saved = const offset_of!(Frame, cw_saved),
                    cw_nearest = const offset_of!(Frame, cw_nearest),
                    cw_chop = const offset_of!(Frame, cw_chop),
                    max = const offset_of!(Frame, max),
                    out("st(0)") _, out("st(1)") _, out("st(2)") _, out("st(3)") _,
                    out("st(4)") _, out("st(5)") _, out("st(6)") _, out("st(7)") _,
                    options(nostack),
                )
            }
        }};
    }

    /// `sqrtFixp` (e.g. `FDKaacEnc_CalcFormFactor`, sf_estim.o 0x208-0x233: `fildl`, `fsqrt`,
    /// `fmuls`, `fnstcw`, `or $0xc00`, `fldcw`, `fistpl`, `fldcw`). For negative `op` the
    /// reference calls `sqrtf` where it cannot prove `op >= 0`; that NaN and the NaN of `fsqrt`
    /// both give `INT_MIN`.
    #[inline]
    pub fn sqrt_fixp(op: i32) -> i32 {
        let mut f = Frame::new(op, 0, [SQRT_FIXP_SCALE, 0.0, 0.0, 0.0, 0.0, 0.0]);
        x87!(
            &mut f,
            "fnstcw word ptr [{f} + {cw_saved}]",
            "fldcw word ptr [{f} + {cw_nearest}]",
            "fild dword ptr [{f} + {a}]",
            "fsqrt",
            "fmul dword ptr [{f} + {k0}]",
            "fldcw word ptr [{f} + {cw_chop}]",
            "fistp dword ptr [{f} + {out}]",
            "fldcw word ptr [{f} + {cw_saved}]",
        );
        f.out
    }

    /// `(float)(1.0 / sqrt(0.5f * (float)op))` of `invSqrtNorm2` (`op != 0`; e.g. `CPns_Apply`,
    /// aacdec_pns.o 0x44c-0x488: `fildl`, `fmuls 0.5f`, `fsqrt`, `fld1`, `fdivp`, `fstps`). The
    /// division is written `fdiv st, st(1)` (1 / root on top) and the root popped afterwards;
    /// the rounded quotient is the same. Negative `op` gives NaN (the reference calls `sqrtf`).
    #[inline]
    pub fn inv_sqrt_norm2_f32(op: i32) -> f32 {
        let mut f = Frame::new(op, 0, [0.5, 0.0, 0.0, 0.0, 0.0, 0.0]);
        x87!(
            &mut f,
            "fnstcw word ptr [{f} + {cw_saved}]",
            "fldcw word ptr [{f} + {cw_nearest}]",
            "fild dword ptr [{f} + {a}]",
            "fmul dword ptr [{f} + {k0}]",
            "fsqrt",
            "fld1",
            "fdiv st, st(1)",
            "fstp dword ptr [{f} + {out}]",
            "fstp st(0)",
            "fldcw word ptr [{f} + {cw_saved}]",
        );
        f32::from_bits(f.out as u32)
    }

    /// `(float)(1.0 / (float)op)` of both `invFixp` (`op != 0`; e.g. `pcmLimiter_Create`,
    /// limiter.o 0xdc-0xea: `fld1`, `fidivl`, `fstps`).
    #[inline]
    pub fn inv_fixp_f32(op: i32) -> f32 {
        let mut f = Frame::new(op, 0, [0.0; 6]);
        x87!(
            &mut f,
            "fnstcw word ptr [{f} + {cw_saved}]",
            "fldcw word ptr [{f} + {cw_nearest}]",
            "fld1",
            "fidiv dword ptr [{f} + {a}]",
            "fstp dword ptr [{f} + {out}]",
            "fldcw word ptr [{f} + {cw_saved}]",
        );
        f32::from_bits(f.out as u32)
    }

    /// `FL2FXCONST_DBL((float)i * c1 * c2)` (the DRC `limDecay` of
    /// `FDK_DRC_Generator_Initialize`, metadata_compressor.o 0xbf-0x16d: `fildl`, `fmuls c1`,
    /// `fmuls c2`, `fucomi` with 0, `fmuls 2^31`, `fadds 0.5f` resp. `fadds -0.5f`, `fucomi`
    /// with `fldl 2147483647.0` resp. `flds -2147483648.0f`, `fistpl`), see
    /// [`super::soft::fl2fxconst_dbl_mul2`].
    #[inline]
    pub fn fl2fxconst_dbl_mul2(i: i32, c1: f32, c2: f32) -> i32 {
        let mut f = Frame::new(i, 0, [c1, c2, 2147483648.0, 0.5, -0.5, -2147483648.0]);
        f.max = 2147483647.0;
        x87!(
            &mut f,
            "fnstcw word ptr [{f} + {cw_saved}]",
            "fldcw word ptr [{f} + {cw_nearest}]",
            "fild dword ptr [{f} + {a}]",
            "fmul dword ptr [{f} + {k0}]",
            "fmul dword ptr [{f} + {k1}]",
            // val >= 0 <=> 0 <= val: CF or ZF (fmul leaves EFLAGS unchanged)
            "fldz",
            "fucomip st, st(1)",
            "fmul dword ptr [{f} + {k2}]",
            "jbe 2f",
            // negative: v = val * 2^31 + -0.5f, -2147483648.0f >= v saturates (not CF)
            "fadd dword ptr [{f} + {k4}]",
            "fld dword ptr [{f} + {k5}]",
            "fucomip st, st(1)",
            "jae 4f",
            "jmp 3f",
            // non-negative: v = val * 2^31 + 0.5f, 2147483647.0 <= v saturates (CF or ZF)
            "2:",
            "fadd dword ptr [{f} + {k3}]",
            "fld qword ptr [{f} + {max}]",
            "fucomip st, st(1)",
            "jbe 5f",
            "3:",
            "fldcw word ptr [{f} + {cw_chop}]",
            "fistp dword ptr [{f} + {out}]",
            "mov dword ptr [{f} + {which}], 0",
            "jmp 6f",
            "4:",
            "fstp st(0)",
            "mov dword ptr [{f} + {which}], 1",
            "jmp 6f",
            "5:",
            "fstp st(0)",
            "mov dword ptr [{f} + {which}], 2",
            "6:",
            "fldcw word ptr [{f} + {cw_saved}]",
        );
        match f.which {
            0 => f.out,
            1 => i32::MIN,
            _ => i32::MAX,
        }
    }

    /// `(INT)((float)op * k)` (`FDKaacEnc_CalcGaussWindow`, aacenc_tns.o 0x5c1-0x5ee: `fildl`,
    /// `fmuls 1000.f`, `fistpl` with the rounding mode "toward zero").
    #[inline]
    pub fn mul_trunc(op: i32, k: f32) -> i32 {
        let mut f = Frame::new(op, 0, [k, 0.0, 0.0, 0.0, 0.0, 0.0]);
        x87!(
            &mut f,
            "fnstcw word ptr [{f} + {cw_saved}]",
            "fldcw word ptr [{f} + {cw_nearest}]",
            "fild dword ptr [{f} + {a}]",
            "fmul dword ptr [{f} + {k0}]",
            "fldcw word ptr [{f} + {cw_chop}]",
            "fistp dword ptr [{f} + {out}]",
            "fldcw word ptr [{f} + {cw_saved}]",
        );
        f.out
    }

    /// `(float)a < hi * (float)l && (float)a > lo * (float)l` (`calcPeCorrection` inlined in
    /// `FDKaacEnc_DistributeBits`, adj_thr.o 0x191e-0x194e: `fildl`, `fmuls 1.5f`, `fucomip`,
    /// `fmuls 0.7f`, `fucomip`).
    #[inline]
    pub fn between_products(a: i32, l: i32, lo: f32, hi: f32) -> bool {
        let mut f = Frame::new(a, l, [lo, hi, 0.0, 0.0, 0.0, 0.0]);
        x87!(
            &mut f,
            "fnstcw word ptr [{f} + {cw_saved}]",
            "fldcw word ptr [{f} + {cw_nearest}]",
            "mov dword ptr [{f} + {out}], 0",
            "mov dword ptr [{f} + {which}], 0",
            // a < hi * l (CF)
            "fild dword ptr [{f} + {b}]",
            "fmul dword ptr [{f} + {k1}]",
            "fild dword ptr [{f} + {a}]",
            "fucomip st, st(1)",
            "fstp st(0)",
            "jae 2f",
            "mov dword ptr [{f} + {out}], 1",
            "2:",
            // a > lo * l (neither CF nor ZF)
            "fild dword ptr [{f} + {b}]",
            "fmul dword ptr [{f} + {k0}]",
            "fild dword ptr [{f} + {a}]",
            "fucomip st, st(1)",
            "fstp st(0)",
            "jbe 3f",
            "mov dword ptr [{f} + {which}], 1",
            "3:",
            "fldcw word ptr [{f} + {cw_saved}]",
        );
        f.out != 0 && f.which != 0
    }

    /// The x87 control word (`fnstcw`).
    pub fn control_word() -> u16 {
        let mut cw = 0u16;
        // SAFETY: stores the control word to `cw`.
        unsafe { asm!("fnstcw word ptr [{p}]", p = in(reg) &mut cw as *mut u16, options(nostack)) };
        cw
    }

    /// Loads the x87 control word (`fldcw`).
    ///
    /// # Safety
    ///
    /// The caller must restore the previous control word before code that relies on it runs
    /// (Rust code on targets without SSE2 computes `f32`/`f64` on the x87 unit).
    pub unsafe fn set_control_word(cw: u16) {
        // SAFETY: loads the control word from `cw`.
        unsafe { asm!("fldcw word ptr [{p}]", p = in(reg) &cw as *const u16, options(nostack)) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_conversions() {
        for v in [0, 1, -1, 3, 0x7fff_ffff, i32::MIN, 12345678] {
            assert_eq!(F80::from_i32(v).to_i32_trunc(), v);
            assert_eq!(F80::from_i32(v).to_f64(), v as f64);
        }
        for v in [0.5f32, -1.25, 46340.9492, 1e-20, 3.0e30] {
            assert_eq!(F80::from_f32(v).to_f32(), v);
        }
        assert_eq!(F80::from_f64(2147483648.0).to_i32_trunc(), i32::MIN);
        assert_eq!(F80::from_f64(-2147483648.0).to_i32_trunc(), i32::MIN);
        assert_eq!(F80::from_f64(-2147483647.9).to_i32_trunc(), -2147483647);
        assert_eq!(F80::NAN.to_i32_trunc(), i32::MIN);
    }

    #[test]
    fn rounding_to_f32_and_f64() {
        // 1 + 2^-24 is a tie in float (to even: 1), 1 + 3 * 2^-25 rounds up
        let a = F80::ONE.mul(F80::from_f64(1.0 + 2f64.powi(-24)));
        assert_eq!(a.to_f32(), 1.0);
        let b = F80::from_f64(1.0 + 3.0 * 2f64.powi(-25));
        assert_eq!(b.to_f32(), 1.0 + 2f32.powi(-23));
        assert_eq!(F80::from_i32(16777217).to_f32(), 16777216.0);
        assert_eq!(F80::from_i32(16777219).to_f32(), 16777220.0);
        assert_eq!(F80::from_i64((1 << 53) + 1).to_f64(), (1u64 << 53) as f64);
    }

    #[test]
    fn addition() {
        let d = |v: f64| F80::from_f64(v);
        assert_eq!(d(1.5).add(d(2.25)).to_f64(), 3.75);
        assert_eq!(d(1.5).sub(d(2.25)).to_f64(), -0.75);
        assert_eq!(d(1.0).sub(d(1.0)).to_f64(), 0.0);
        // 2^64 + 1 is rounded to 2^64 (ties to even), 2^64 + 3 to 2^64 + 4 (65 bits)
        let two64 = d(2f64.powi(64));
        assert_eq!(two64.add(d(1.0)), two64);
        assert_eq!(two64.add(d(3.0)).sub(two64).to_f64(), 4.0);
        // 1 - 2^-70 rounds to 1, 1 - 2^-64 is exact
        assert_eq!(d(1.0).sub(d(2f64.powi(-70))), d(1.0));
        assert_eq!(d(1.0).sub(d(2f64.powi(-64))).sub(d(1.0)).to_f64(), -(2f64.powi(-64)));
        assert_eq!(d(1e19).to_u64_trunc(), 10_000_000_000_000_000_000);
        assert_eq!(d(12345.99).to_u64_trunc(), 12345);
    }

    #[test]
    fn operations_match_f64_where_the_result_is_exact() {
        let d = |v: f64| F80::from_f64(v);
        assert_eq!(d(3.0).mul(d(-0.5)).to_f64(), -1.5);
        assert_eq!(d(1.0).div(d(4.0)).to_f64(), 0.25);
        assert_eq!(d(2.25).sqrt().to_f64(), 1.5);
        assert_eq!(d(1e-6).mul(d(1e6)).div(d(1e-6)).to_f64(), 1e6);
        assert!(d(-2.0).sqrt().is_nan());
    }

    /// The extended precision results rounded to `double` agree with the correctly rounded
    /// `f64` results except for rare double rounding cases.
    #[test]
    fn operations_round_like_ieee_extended() {
        let mut x = 0x1234_5678_9abc_def1u64;
        let mut same = 0;
        for _ in 0..10000 {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            let a = (x >> 11) as f64 / (1u64 << 40) as f64 + 0.5;
            let b = (x & 0xfff_ffff) as f64 + 1.0;
            same += (F80::from_f64(a).div(F80::from_f64(b)).to_f64() == a / b) as u32;
            same += (F80::from_f64(a).mul(F80::from_f64(b)).to_f64() == a * b) as u32;
            same += (F80::from_f64(a).sqrt().to_f64() == a.sqrt()) as u32;
        }
        assert!(same >= 29990, "{same}");
    }

    /// xorshift64
    fn next(x: &mut u64) -> u64 {
        *x ^= *x << 13;
        *x ^= *x >> 7;
        *x ^= *x << 17;
        *x
    }

    /// Operands of the helpers: edge cases, powers of two and their neighbours, perfect squares
    /// (exact `sqrtFixp` products for multiples of 256) and `n` random values of every magnitude
    /// and both signs.
    fn operands(seed: u64, n: usize) -> Vec<i32> {
        let mut v = vec![0, 1, 2, 3, 4, 5, 7, 9, i32::MAX, i32::MAX - 1, i32::MIN, i32::MIN + 1];
        v.extend([-1, -2, -3, 0x3fff_ffff, 0x4000_0001, 0xffff, 0x10001, 46341 * 46340]);
        for k in 0..31 {
            let p = 1i32 << k;
            v.extend([p, p + 1, p - 1, -p, -p - 1]);
        }
        for r in [1, 2, 3, 181, 255, 256, 257, 512, 1000, 4096, 23170, 46080, 46339, 46340] {
            v.extend([r * r - 1, r * r, r * r + 1]);
        }
        let mut x = seed | 1;
        for _ in 0..n {
            let r = next(&mut x);
            let m = ((r >> 33) as u32 >> (r % 32)) as i32;
            v.push(if r & (1 << 20) != 0 { m } else { m.wrapping_neg() });
        }
        v
    }

    /// `a` and `b` are the same `float`, or both NaN.
    fn same_f32(a: f32, b: f32) -> bool {
        a.to_bits() == b.to_bits() || (a.is_nan() && b.is_nan())
    }

    /// The `f64` paths equal the model; the fallback to the model is rare.
    #[test]
    fn fast_matches_soft() {
        let ops = operands(0x5eed_0001, if cfg!(debug_assertions) { 100_000 } else { 400_000 });
        let mut fallbacks = [0usize; 3];
        for &op in &ops {
            let s = soft::sqrt_fixp(op);
            assert_eq!(fast::sqrt_fixp(op), s, "sqrt_fixp({op:#x})");
            match fast::try_sqrt_fixp(op) {
                Some(v) => assert_eq!(v, s, "try_sqrt_fixp({op:#x})"),
                None => fallbacks[0] += 1,
            }
            if op != 0 {
                let s = soft::inv_sqrt_norm2_f32(op);
                assert!(same_f32(fast::inv_sqrt_norm2_f32(op), s), "inv_sqrt_norm2_f32({op:#x})");
                match fast::try_inv_sqrt_norm2_f32(op) {
                    Some(v) => assert_eq!(v.to_bits(), s.to_bits(), "{op:#x}"),
                    None => fallbacks[1] += (op > 0) as usize,
                }
                let s = soft::inv_fixp_f32(op);
                assert_eq!(fast::inv_fixp_f32(op).to_bits(), s.to_bits(), "inv_fixp_f32({op:#x})");
                match fast::try_inv_fixp_f32(op) {
                    Some(v) => assert_eq!(v.to_bits(), s.to_bits(), "{op:#x}"),
                    None => fallbacks[2] += 1,
                }
            }
        }
        // the exact products of sqrtFixp (perfect squares of multiples of 256) fall back
        assert_eq!(fast::try_sqrt_fixp(65536), None);
        eprintln!("fast_matches_soft: fallbacks {fallbacks:?} of {} operands", ops.len());
        assert!(fallbacks.iter().all(|&f| f < 200), "{fallbacks:?} of {}", ops.len());
    }

    /// The `f64` paths and the model for every `int` operand. Slow (2^32 operands; threads:
    /// `X87_TEST_THREADS`, default 2): run explicitly with `--ignored` in the release profile.
    #[test]
    #[ignore = "exhaustive, run with --ignored in the release profile"]
    fn fast_matches_soft_exhaustive() {
        let threads: u64 =
            std::env::var("X87_TEST_THREADS").ok().and_then(|t| t.parse().ok()).unwrap_or(2);
        let handles: Vec<_> = (0..threads)
            .map(|t| {
                std::thread::spawn(move || {
                    let mut fallbacks = [0u64; 3];
                    for v in (t << 32) / threads..((t + 1) << 32) / threads {
                        let op = v as u32 as i32;
                        let s = soft::sqrt_fixp(op);
                        match fast::try_sqrt_fixp(op) {
                            Some(f) => assert_eq!(f, s, "sqrt_fixp({op:#x})"),
                            None => fallbacks[0] += 1,
                        }
                        if op > 0 {
                            let s = soft::inv_sqrt_norm2_f32(op);
                            match fast::try_inv_sqrt_norm2_f32(op) {
                                Some(f) => assert_eq!(
                                    f.to_bits(),
                                    s.to_bits(),
                                    "inv_sqrt_norm2_f32({op:#x})"
                                ),
                                None => fallbacks[1] += 1,
                            }
                        }
                        if op != 0 {
                            let s = soft::inv_fixp_f32(op);
                            match fast::try_inv_fixp_f32(op) {
                                Some(f) => {
                                    assert_eq!(f.to_bits(), s.to_bits(), "inv_fixp_f32({op:#x})")
                                }
                                None => fallbacks[2] += 1,
                            }
                        }
                    }
                    fallbacks
                })
            })
            .collect();
        let mut fallbacks = [0u64; 3];
        for h in handles {
            for (a, b) in fallbacks.iter_mut().zip(h.join().unwrap()) {
                *a += b;
            }
        }
        eprintln!("fast_matches_soft_exhaustive: fallbacks {fallbacks:?} of 2^32 operands");
    }

    /// The model of `FL2FXCONST_DBL((float)i * c1 * c2)` at the DRC `limDecay` constants and at
    /// the saturation bounds.
    #[test]
    fn fl2fxconst_dbl_mul2_soft() {
        let (c1, c2) = (0.006f32 / 256.0, 1.0f32 / 1024.0);
        // 1024 * (0.006f / 256) / 1024 * 2^31 + 0.5 = 50332.148...
        assert_eq!(soft::fl2fxconst_dbl_mul2(1024, c1, c2), 50332);
        assert_eq!(soft::fl2fxconst_dbl_mul2(-1024, c1, c2), -50332);
        assert_eq!(soft::fl2fxconst_dbl_mul2(0, c1, c2), 0);
        assert_eq!(soft::fl2fxconst_dbl_mul2(1, 1.0, 1.0), i32::MAX);
        assert_eq!(soft::fl2fxconst_dbl_mul2(-1, 1.0, 1.0), i32::MIN);
        assert_eq!(soft::fl2fxconst_dbl_mul2(1, 0.5, 1.0), 0x4000_0000);
        assert_eq!(soft::fl2fxconst_dbl_mul2(-1, 0.5, 1.0), -0x4000_0000);
    }

    /// The x87 instructions give the results of the model, for every control word of the
    /// caller, and restore the caller's control word.
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    mod hw_tests {
        use super::super::{fast, hw, soft};
        use super::{next, operands, same_f32};

        /// A random `float` constant of moderate magnitude (2^-27 .. 2^12), either sign.
        fn constant(b: u64) -> f32 {
            f32::from_bits(((b as u32) & 0x807f_ffff) | ((100 + (b >> 40) as u32 % 40) << 23))
        }

        /// Every x87 helper against the model for one operand.
        fn check(op: i32, x: &mut u64) {
            assert_eq!(hw::sqrt_fixp(op), soft::sqrt_fixp(op), "sqrt_fixp({op:#x})");
            if op != 0 {
                let (h, s) = (hw::inv_sqrt_norm2_f32(op), soft::inv_sqrt_norm2_f32(op));
                assert!(same_f32(h, s), "inv_sqrt_norm2_f32({op:#x}): {h:e} {s:e}");
                let (h, s) = (hw::inv_fixp_f32(op), soft::inv_fixp_f32(op));
                assert_eq!(h.to_bits(), s.to_bits(), "inv_fixp_f32({op:#x}): {h:e} {s:e}");
            }
            // (INT)((float)op * k): exact and rounded products, in and out of range
            let r = next(x);
            let ks = [1000.0f32, 0.5, 46340.9492, 3.0e-5, 1.0 / 3.0, 7.7e9, -1.1, constant(r)];
            let k = ks[(r >> 32) as usize % ks.len()];
            assert_eq!(hw::mul_trunc(op, k), soft::mul_trunc(op, k), "mul_trunc({op:#x}, {k:e})");
            // a < hi * l && a > lo * l next to the bounds
            let l = (next(x) as i32) >> (next(x) % 32);
            let (lo, hi) = (0.7f32, 1.5f32);
            for a in [
                (l as f64 * 0.7) as i32,
                (l as f64 * 1.5) as i32,
                op,
                (op as f64 * 0.7) as i32,
                (op as f64 * 1.5) as i32,
            ] {
                for d in -1..=1 {
                    let a = a.wrapping_add(d);
                    assert_eq!(
                        hw::between_products(a, l, lo, hi),
                        soft::between_products(a, l, lo, hi),
                        "between_products({a}, {l})"
                    );
                    assert_eq!(
                        hw::between_products(a, op, lo, hi),
                        soft::between_products(a, op, lo, hi),
                        "between_products({a}, {op})"
                    );
                }
            }
            // FL2FXCONST_DBL((float)i * c1 * c2): limDecay constants, random constants,
            // saturation
            let r = next(x);
            let cs = [
                (0.006f32 / 256.0, 1.0f32 / 1024.0),
                (constant(r), constant(r >> 7)),
                (1.0, 1.0),
                (0.5, 1.0),
            ];
            for (c1, c2) in cs {
                assert_eq!(
                    hw::fl2fxconst_dbl_mul2(op, c1, c2),
                    soft::fl2fxconst_dbl_mul2(op, c1, c2),
                    "fl2fxconst_dbl_mul2({op:#x}, {c1:e}, {c2:e})"
                );
            }
        }

        #[test]
        fn hw_matches_soft() {
            let mut x = 0x5eed_0003u64;
            let n = if cfg!(debug_assertions) { 100_000 } else { 400_000 };
            for op in operands(0x5eed_0002, n) {
                check(op, &mut x);
            }
            for i in -70_000..=70_000 {
                check(i, &mut x);
            }
        }

        /// The results do not depend on the caller's precision and rounding control, and the
        /// caller's control word is restored.
        #[test]
        fn hw_control_word() {
            let saved = hw::control_word();
            let mut x = 0x5eed_0005u64;
            let ops = operands(0x5eed_0004, 2_000);
            let (c1, c2) = (0.006f32 / 256.0, 1.0f32 / 1024.0);
            // extended, double and single precision; the four rounding modes
            for cw in [0x037f, 0x027f, 0x007f, 0x0f7f, 0x077f, 0x0b7f, 0x0c7f] {
                let mut res = Vec::with_capacity(ops.len());
                // SAFETY: only the x87 helpers run until the control word is restored (the
                // vector has its capacity, the integer code does not use the x87 unit).
                unsafe { hw::set_control_word(cw) };
                for &op in &ops {
                    let nz = op != 0;
                    res.push((
                        hw::sqrt_fixp(op),
                        if nz { hw::inv_sqrt_norm2_f32(op).to_bits() } else { 0 },
                        if nz { hw::inv_fixp_f32(op).to_bits() } else { 0 },
                        hw::mul_trunc(op, 0.3),
                        hw::between_products(op, 1234567, 0.7, 1.5),
                        hw::fl2fxconst_dbl_mul2(op, c1, c2),
                        hw::control_word(),
                    ));
                }
                // SAFETY: restores the control word of the test thread.
                unsafe { hw::set_control_word(saved) };
                for (&op, r) in ops.iter().zip(res) {
                    assert_eq!(r.6, cw, "control word {cw:#x} not restored");
                    assert_eq!(r.0, soft::sqrt_fixp(op), "{cw:#x} sqrt_fixp({op:#x})");
                    if op != 0 {
                        let s = soft::inv_sqrt_norm2_f32(op);
                        let m = "inv_sqrt_norm2_f32";
                        assert!(same_f32(f32::from_bits(r.1), s), "{cw:#x} {m}({op:#x})");
                        let s = soft::inv_fixp_f32(op).to_bits();
                        assert_eq!(r.2, s, "{cw:#x} inv_fixp_f32({op:#x})");
                    }
                    assert_eq!(r.3, soft::mul_trunc(op, 0.3), "{cw:#x} mul_trunc({op:#x})");
                    let s = soft::between_products(op, 1234567, 0.7, 1.5);
                    assert_eq!(r.4, s, "{cw:#x} between_products({op:#x})");
                    let s = soft::fl2fxconst_dbl_mul2(op, c1, c2);
                    assert_eq!(r.5, s, "{cw:#x} fl2fxconst_dbl_mul2({op:#x})");
                }
                check(1, &mut x);
            }
            assert_eq!(hw::control_word(), saved);
        }

        /// The x87 instructions and the `f64` paths (with their fallback to the model) for every
        /// `int` operand. Slow (2^32 operands, 4 threads): run explicitly with `--ignored` in the
        /// release profile.
        #[test]
        #[ignore = "exhaustive, run with --ignored in the release profile"]
        fn hw_matches_fast_exhaustive() {
            let threads = 4u64;
            let handles: Vec<_> = (0..threads)
                .map(|t| {
                    std::thread::spawn(move || {
                        for v in (t << 30)..((t + 1) << 30) {
                            let op = v as u32 as i32;
                            let (h, f) = (hw::sqrt_fixp(op), fast::sqrt_fixp(op));
                            assert_eq!(h, f, "sqrt_fixp({op:#x})");
                            if op != 0 {
                                let h = hw::inv_sqrt_norm2_f32(op);
                                let f = fast::inv_sqrt_norm2_f32(op);
                                assert!(same_f32(h, f), "inv_sqrt_norm2_f32({op:#x})");
                                let (h, f) = (hw::inv_fixp_f32(op), fast::inv_fixp_f32(op));
                                assert_eq!(h.to_bits(), f.to_bits(), "inv_fixp_f32({op:#x})");
                            }
                        }
                    })
                })
                .collect();
            for h in handles {
                h.join().unwrap();
            }
        }
    }
}
