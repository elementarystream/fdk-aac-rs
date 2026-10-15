// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libFDK/include/common_fix.h.

//! Port of `common_fix.h`: the fixed point types, conversion macros and the overloaded
//! arithmetic primitives (`fMult`, `fMultDiv2`, `fMultAdd`, `fPow2`, `fAbs`, `fNorm`, `fMin`, ...).
//!
//! The overload sets are generic functions that dispatch on the operand types (`i32` =
//! `FIXP_DBL`, `i16` = `FIXP_SGL`), exactly like C++ overload resolution: a wrong operand type
//! selects a different overload with a different rounding.
//!
//! Like `common_fix.h`, which includes `fixmul.h`, `fixmadd.h`, `abs.h`, `clz.h`,
//! `fixminmax.h`, `cplx_mul.h` and `fixpoint_math.h` (and through it `scale.h`), this module
//! re-exports those modules.

pub use crate::abs::*;
pub use crate::clz::*;
pub use crate::cplx_mul::*;
pub use crate::fixmadd::*;
pub use crate::fixminmax::*;
pub use crate::fixmul::*;
pub use crate::fixpoint_math::*;
pub use crate::scale::*;

use crate::machine_type::IntPcm;

/// The ARM code paths of the reference are compiled (`__arm__` and `__ARM_ARCH_8__`, which
/// `FDK_archdef.h` defines for `__aarch64__`): the inline assembly primitives, `scale_arm.cpp`.
pub(crate) const ARCH_ARM: bool = matches!(crate::ARCH, crate::Arch::Aarch64);

/// The `x86/*.h` headers of the reference are compiled (`__x86__`): in particular the `float`
/// implementations of `sqrtFixp`, `invSqrtNorm2`, `invFixp` and the 64 bit division of
/// `schur_div` (`x86/fixpoint_math_x86.h`).
pub(crate) const ARCH_X86: bool = matches!(crate::ARCH, crate::Arch::X86 | crate::Arch::X86_32);

/// The 32-bit x86 build (`ref-i686`): `x86/fixpoint_math_x86.h` computes on the x87 unit with
/// extended precision (see [`crate::x87`]) instead of SSE.
pub(crate) const ARCH_X86_32: bool = matches!(crate::ARCH, crate::Arch::X86_32);

// ---------------------------------------------------------------------------------------------
// Types and constants
// ---------------------------------------------------------------------------------------------

/// `FRACT_BITS`: single precision
pub const FRACT_BITS: i32 = 16;
/// `DFRACT_BITS`: double precision
pub const DFRACT_BITS: i32 = 32;
/// `ACCU_BITS`: double precision plus overflow
pub const ACCU_BITS: i32 = 40;

/// `SGL_MASK`
pub const SGL_MASK: u32 = (1u32 << FRACT_BITS) - 1;
/// `MAX_SHIFT_SGL`: maximum possible shift for `FIXP_SGL` values
pub const MAX_SHIFT_SGL: i32 = FRACT_BITS - 1;
/// `MAX_SHIFT_DBL`: maximum possible shift for `FIXP_DBL` values
pub const MAX_SHIFT_DBL: i32 = DFRACT_BITS - 1;

/// `FRACT_FIX_SCALE` (not a saturation limit)
pub const FRACT_FIX_SCALE: i64 = 1i64 << (FRACT_BITS - 1);
/// `DFRACT_FIX_SCALE` (not a saturation limit)
pub const DFRACT_FIX_SCALE: i64 = 1i64 << (DFRACT_BITS - 1);

/// `MAXVAL_SGL` (an `int`)
pub const MAXVAL_SGL: i32 = 0x0000_7FFF;
/// `MINVAL_SGL` (an `int`)
pub const MINVAL_SGL: i32 = 0xFFFF_8000u32 as i32;
/// `MAXVAL_DBL`
pub const MAXVAL_DBL: i32 = 0x7FFF_FFFF;
/// `MINVAL_DBL`
pub const MINVAL_DBL: i32 = 0x8000_0000u32 as i32;
/// `MINVAL_DBL_CONST`
pub const MINVAL_DBL_CONST: i32 = MINVAL_DBL;
/// `MINVAL_SGL_CONST`
pub const MINVAL_SGL_CONST: i32 = MINVAL_SGL;

/// `FIXP_SGL` (`SHORT`)
pub type FixpSgl = i16;
/// `FIXP_DBL` (`LONG`)
pub type FixpDbl = i32;

/// `FIXP_PCM`: `FIXP_SGL`, because `SAMPLE_BITS == FRACT_BITS` (`machine_type.h`).
pub type FixpPcm = FixpSgl;
/// `MAXVAL_FIXP_PCM`
pub const MAXVAL_FIXP_PCM: i32 = MAXVAL_SGL;
/// `MINVAL_FIXP_PCM`
pub const MINVAL_FIXP_PCM: i32 = MINVAL_SGL;

/// `FIXP_SPK`: a union of `struct { FIXP_SGL re, im; } v` and `LONG w`. The port keeps the
/// vector representation; code that uses `w` is translated to copy both fields.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(C)]
pub struct FixpSpk {
    pub re: FixpSgl,
    pub im: FixpSgl,
}

/// `FIXP_DPK`: a union of `struct { FIXP_DBL re, im; } v` and `INT64 w`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(C)]
pub struct FixpDpk {
    pub re: FixpDbl,
    pub im: FixpDbl,
}

impl FixpSpk {
    pub const fn new(re: FixpSgl, im: FixpSgl) -> Self {
        Self { re, im }
    }
}

impl FixpDpk {
    pub const fn new(re: FixpDbl, im: FixpDbl) -> Self {
        Self { re, im }
    }
}

// ---------------------------------------------------------------------------------------------
// Conversion macros
// ---------------------------------------------------------------------------------------------

/// `FX_DBL2FXCONST_SGL(val)`: rounded conversion of a `FIXP_DBL` to `FIXP_SGL` that saturates
/// positive values.
#[inline(always)]
pub const fn fx_dbl2fxconst_sgl(val: FixpDbl) -> FixpSgl {
    if ((val >> (DFRACT_BITS - FRACT_BITS - 1)) + 1) > ((1i32 << FRACT_BITS) - 1) && val > 0 {
        ((1i32 << (FRACT_BITS - 1)) - 1) as i16
    } else {
        (((val >> (DFRACT_BITS - FRACT_BITS - 1)) + 1) >> 1) as i16
    }
}

/// `FL2FXCONST_SGL(val)`: conversion of a floating point constant to `FIXP_SGL`, computed in
/// `double`.
///
/// C evaluates `val` in the precision of its expression (`float` for `f`-suffixed literals);
/// callers reproduce that by computing the argument in `f32` and converting it with `as f64`.
#[inline(always)]
pub const fn fl2fxconst_sgl(val: f64) -> FixpSgl {
    let s = FRACT_FIX_SCALE as f64;
    if val >= 0.0 {
        if val * s + 0.5 >= MAXVAL_SGL as f64 {
            MAXVAL_SGL as i16
        } else {
            (val * s + 0.5) as i16
        }
    } else if val * s - 0.5 <= MINVAL_SGL_CONST as f64 {
        MINVAL_SGL_CONST as i16
    } else {
        (val * s - 0.5) as i16
    }
}

/// `FL2FXCONST_DBL(val)`: conversion of a floating point constant to `FIXP_DBL`, computed in
/// `double`. See [`fl2fxconst_sgl`] for the precision of the argument.
#[inline(always)]
pub const fn fl2fxconst_dbl(val: f64) -> FixpDbl {
    let s = DFRACT_FIX_SCALE as f64;
    if val >= 0.0 {
        if val * s + 0.5 >= MAXVAL_DBL as f64 {
            MAXVAL_DBL
        } else {
            (val * s + 0.5) as i32
        }
    } else if val * s - 0.5 <= MINVAL_DBL_CONST as f64 {
        MINVAL_DBL_CONST
    } else {
        (val * s - 0.5) as i32
    }
}

/// `FL2FXCONST_SPC(val)`: `FL2FXCONST_DBL`
#[inline(always)]
pub const fn fl2fxconst_spc(val: f64) -> FixpDbl {
    fl2fxconst_dbl(val)
}

/// `FL2FX_SGL(val)`: run-time conversion of a `float` to `FIXP_SGL`, without overflow check.
/// The conversion of out-of-range values is undefined in C; `as` saturates.
#[inline(always)]
pub fn fl2fx_sgl(val: f32) -> FixpSgl {
    // Only in-range values are defined in C (the library converts constants only).
    if ARCH_X86_32 {
        return fl2fx_x87(val, FRACT_FIX_SCALE as f32) as i16;
    }
    if val > 0.0 {
        (val * FRACT_FIX_SCALE as f32 + 0.5) as i32 as i16
    } else {
        (val * FRACT_FIX_SCALE as f32 - 0.5) as i32 as i16
    }
}

/// `FL2FX_DBL(val)`: run-time conversion of a `float` to `FIXP_DBL`, without overflow check.
/// The conversion of out-of-range values is undefined in C; `as` saturates.
#[inline(always)]
pub fn fl2fx_dbl(val: f32) -> FixpDbl {
    if ARCH_X86_32 {
        return fl2fx_x87(val, DFRACT_FIX_SCALE as f32);
    }
    if val > 0.0 {
        (val * DFRACT_FIX_SCALE as f32 + 0.5) as i32
    } else {
        (val * DFRACT_FIX_SCALE as f32 - 0.5) as i32
    }
}

/// `FL2FX_SGL`/`FL2FX_DBL` on the x87 unit (`x86_32`): `val * scale +- 0.5f` in extended
/// precision (exact for a `float` in range), truncated (`fistp`).
fn fl2fx_x87(val: f32, scale: f32) -> i32 {
    use crate::x87::F80;
    let p = F80::from_f32(val).mul(F80::from_f32(scale));
    let half = F80::from_f32(if val > 0.0 { 0.5 } else { -0.5 });
    p.add(half).to_i32_trunc()
}

/// `FL2FX_SPC(val)`: `FL2FX_DBL`
#[inline(always)]
pub fn fl2fx_spc(val: f32) -> FixpDbl {
    fl2fx_dbl(val)
}

/// `FX_SGL2FX_DBL(val)`: `(LONG)val << 16`
#[inline(always)]
pub const fn fx_sgl2fx_dbl(val: FixpSgl) -> FixpDbl {
    (val as i32) << (DFRACT_BITS - FRACT_BITS)
}

/// `FX_DBL2FX_SGL(val)`: `(FIXP_SGL)(val >> 16)` (truncating)
#[inline(always)]
pub const fn fx_dbl2fx_sgl(val: FixpDbl) -> FixpSgl {
    (val >> (DFRACT_BITS - FRACT_BITS)) as i16
}

/// `FX_DBL2FL(val)`: `(float)((double)val / 2^31)`
#[inline(always)]
pub fn fx_dbl2fl(val: FixpDbl) -> f32 {
    (val as f64 / DFRACT_FIX_SCALE as f64) as f32
}

/// `FX_DBL2DOUBLE(val)`: `(double)val / 2^31`
#[inline(always)]
pub fn fx_dbl2double(val: FixpDbl) -> f64 {
    val as f64 / DFRACT_FIX_SCALE as f64
}

/// `FX_PCM2FX_DBL(x)`: `FX_SGL2FX_DBL((FIXP_SGL)(x))` (`SAMPLE_BITS == 16`)
#[inline(always)]
pub const fn fx_pcm2fx_dbl(x: IntPcm) -> FixpDbl {
    fx_sgl2fx_dbl(x)
}

/// `FX_DBL2FX_PCM(x)`: `FX_DBL2FX_SGL(x)` (`SAMPLE_BITS == 16`)
#[inline(always)]
pub const fn fx_dbl2fx_pcm(x: FixpDbl) -> IntPcm {
    fx_dbl2fx_sgl(x)
}

// ---------------------------------------------------------------------------------------------
// fMult, fMultDiv2, fMultBitExact, fMultDiv2BitExact, fPow2, fPow2Div2
// ---------------------------------------------------------------------------------------------

/// Overloads of `fMult` and `fMultDiv2`, selected by the types of `a` (`Self`) and `b` (`B`).
pub trait FMult<B>: Copy {
    fn f_mult(self, b: B) -> FixpDbl;
    fn f_mult_div2(self, b: B) -> FixpDbl;
}

impl FMult<FixpSgl> for FixpSgl {
    #[inline(always)]
    fn f_mult(self, b: i16) -> i32 {
        fixmul_ss(self, b)
    }
    #[inline(always)]
    fn f_mult_div2(self, b: i16) -> i32 {
        fixmuldiv2_ss(self, b)
    }
}

impl FMult<FixpDbl> for FixpSgl {
    #[inline(always)]
    fn f_mult(self, b: i32) -> i32 {
        fixmul_sd(self, b)
    }
    #[inline(always)]
    fn f_mult_div2(self, b: i32) -> i32 {
        fixmuldiv2_sd(self, b)
    }
}

impl FMult<FixpSgl> for FixpDbl {
    #[inline(always)]
    fn f_mult(self, b: i16) -> i32 {
        fixmul_ds(self, b)
    }
    #[inline(always)]
    fn f_mult_div2(self, b: i16) -> i32 {
        fixmuldiv2_ds(self, b)
    }
}

impl FMult<FixpDbl> for FixpDbl {
    #[inline(always)]
    fn f_mult(self, b: i32) -> i32 {
        fixmul_dd(self, b)
    }
    #[inline(always)]
    fn f_mult_div2(self, b: i32) -> i32 {
        fixmuldiv2_dd(self, b)
    }
}

/// `fMult(a, b)`
#[inline(always)]
pub fn f_mult<A: FMult<B>, B>(a: A, b: B) -> FixpDbl {
    a.f_mult(b)
}

/// `fMultDiv2(a, b)`
#[inline(always)]
pub fn f_mult_div2<A: FMult<B>, B>(a: A, b: B) -> FixpDbl {
    a.f_mult_div2(b)
}

/// Overloads of `fMultBitExact` and `fMultDiv2BitExact` (`FIXP_DBL` x `FIXP_DBL`,
/// `FIXP_SGL` x `FIXP_DBL`, `FIXP_DBL` x `FIXP_SGL`).
pub trait FMultBitExact<B>: Copy {
    fn f_mult_bit_exact(self, b: B) -> FixpDbl;
    fn f_mult_div2_bit_exact(self, b: B) -> FixpDbl;
}

impl FMultBitExact<FixpDbl> for FixpDbl {
    #[inline(always)]
    fn f_mult_bit_exact(self, b: i32) -> i32 {
        fixmul_bit_exact_dd(self, b)
    }
    #[inline(always)]
    fn f_mult_div2_bit_exact(self, b: i32) -> i32 {
        fixmuldiv2_bit_exact_dd(self, b)
    }
}

impl FMultBitExact<FixpDbl> for FixpSgl {
    #[inline(always)]
    fn f_mult_bit_exact(self, b: i32) -> i32 {
        fixmul_bit_exact_sd(self, b)
    }
    #[inline(always)]
    fn f_mult_div2_bit_exact(self, b: i32) -> i32 {
        fixmuldiv2_bit_exact_sd(self, b)
    }
}

impl FMultBitExact<FixpSgl> for FixpDbl {
    #[inline(always)]
    fn f_mult_bit_exact(self, b: i16) -> i32 {
        fixmul_bit_exact_ds(self, b)
    }
    #[inline(always)]
    fn f_mult_div2_bit_exact(self, b: i16) -> i32 {
        fixmuldiv2_bit_exact_ds(self, b)
    }
}

/// `fMultBitExact(a, b)`
#[inline(always)]
pub fn f_mult_bit_exact<A: FMultBitExact<B>, B>(a: A, b: B) -> FixpDbl {
    a.f_mult_bit_exact(b)
}

/// `fMultDiv2BitExact(a, b)`
#[inline(always)]
pub fn f_mult_div2_bit_exact<A: FMultBitExact<B>, B>(a: A, b: B) -> FixpDbl {
    a.f_mult_div2_bit_exact(b)
}

/// Overloads of `fPow2`, `fPow2Div2`, `fPow2Add` and `fPow2AddDiv2`.
pub trait FPow2: Copy {
    fn f_pow2(self) -> FixpDbl;
    fn f_pow2_div2(self) -> FixpDbl;
    fn f_pow2_add(x: FixpDbl, a: Self) -> FixpDbl;
    fn f_pow2_add_div2(x: FixpDbl, a: Self) -> FixpDbl;
}

impl FPow2 for FixpDbl {
    #[inline(always)]
    fn f_pow2(self) -> i32 {
        fixpow2_d(self)
    }
    #[inline(always)]
    fn f_pow2_div2(self) -> i32 {
        fixpow2div2_d(self)
    }
    #[inline(always)]
    fn f_pow2_add(x: i32, a: i32) -> i32 {
        fixpadd_d(x, a)
    }
    #[inline(always)]
    fn f_pow2_add_div2(x: i32, a: i32) -> i32 {
        fixpadddiv2_d(x, a)
    }
}

impl FPow2 for FixpSgl {
    #[inline(always)]
    fn f_pow2(self) -> i32 {
        fixpow2_s(self)
    }
    #[inline(always)]
    fn f_pow2_div2(self) -> i32 {
        fixpow2div2_s(self)
    }
    #[inline(always)]
    fn f_pow2_add(x: i32, a: i16) -> i32 {
        fixpadd_s(x, a)
    }
    #[inline(always)]
    fn f_pow2_add_div2(x: i32, a: i16) -> i32 {
        fixpadddiv2_s(x, a)
    }
}

/// `fPow2(a)`
#[inline(always)]
pub fn f_pow2<A: FPow2>(a: A) -> FixpDbl {
    a.f_pow2()
}

/// `fPow2Div2(a)`
#[inline(always)]
pub fn f_pow2_div2<A: FPow2>(a: A) -> FixpDbl {
    a.f_pow2_div2()
}

/// `fPow2Add(x, a)`: `x + fPow2(a)`
#[inline(always)]
pub fn f_pow2_add<A: FPow2>(x: FixpDbl, a: A) -> FixpDbl {
    A::f_pow2_add(x, a)
}

/// `fPow2AddDiv2(x, a)`: `x + fPow2Div2(a)`
#[inline(always)]
pub fn f_pow2_add_div2<A: FPow2>(x: FixpDbl, a: A) -> FixpDbl {
    A::f_pow2_add_div2(x, a)
}

// ---------------------------------------------------------------------------------------------
// fAbs, fNorm, fNormz
// ---------------------------------------------------------------------------------------------

/// Overloads of `fAbs` / `fixp_abs`.
pub trait FAbs: Copy {
    fn f_abs(self) -> Self;
}

impl FAbs for FixpDbl {
    #[inline(always)]
    fn f_abs(self) -> i32 {
        fixabs_d(self)
    }
}

impl FAbs for FixpSgl {
    #[inline(always)]
    fn f_abs(self) -> i16 {
        fixabs_s(self)
    }
}

/// `fAbs(x)` / `fixp_abs(x)`: `fAbs(MINVAL_DBL) == MINVAL_DBL` (see [`crate::abs`]).
#[inline(always)]
pub fn f_abs<T: FAbs>(x: T) -> T {
    x.f_abs()
}

/// `fixp_abs(x)`: `fAbs(x)`
#[inline(always)]
pub fn fixp_abs<T: FAbs>(x: T) -> T {
    x.f_abs()
}

/// Overloads of `fNormz` (`INT64`, `FIXP_DBL`, `FIXP_SGL`).
pub trait FNormz: Copy {
    fn f_normz(self) -> i32;
}

impl FNormz for i64 {
    #[inline(always)]
    fn f_normz(self) -> i32 {
        let mut clz = fixnormz_d((self >> 32) as i32);
        if clz == 32 {
            clz += fixnormz_d(self as i32);
        }
        clz
    }
}

impl FNormz for FixpDbl {
    #[inline(always)]
    fn f_normz(self) -> i32 {
        fixnormz_d(self)
    }
}

impl FNormz for FixpSgl {
    #[inline(always)]
    fn f_normz(self) -> i32 {
        fixnormz_s(self)
    }
}

/// `fNormz(x)`: leading zero bits.
#[inline(always)]
pub fn f_normz<T: FNormz>(x: T) -> i32 {
    x.f_normz()
}

/// Overloads of `fNorm` (`FIXP_DBL`, `FIXP_SGL`).
pub trait FNorm: Copy {
    fn f_norm(self) -> i32;
}

impl FNorm for FixpDbl {
    #[inline(always)]
    fn f_norm(self) -> i32 {
        fixnorm_d(self)
    }
}

impl FNorm for FixpSgl {
    #[inline(always)]
    fn f_norm(self) -> i32 {
        fixnorm_s(self)
    }
}

/// `fNorm(x)`: redundant sign bits.
#[inline(always)]
pub fn f_norm<T: FNorm>(x: T) -> i32 {
    x.f_norm()
}

/// `CntLeadingZeros(x)`: `fixnormz_D(x)`
#[inline(always)]
pub const fn cnt_leading_zeros(x: i32) -> i32 {
    fixnormz_d(x)
}

/// `CountLeadingBits(x)`: `fixnorm_D(x)`
#[inline(always)]
pub const fn count_leading_bits(x: i32) -> i32 {
    fixnorm_d(x)
}

// ---------------------------------------------------------------------------------------------
// fMultAdd, fMultAddDiv2, fMultSub, fMultSubDiv2 and the BitExact variants
// ---------------------------------------------------------------------------------------------

/// Overloads of `fMultAdd`, `fMultAddDiv2`, `fMultSub` and `fMultSubDiv2`, selected by the types
/// of the factors `a` (`Self`) and `b` (`B`).
pub trait FMultAdd<B>: Copy {
    fn madd(x: FixpDbl, a: Self, b: B) -> FixpDbl;
    fn madd_div2(x: FixpDbl, a: Self, b: B) -> FixpDbl;
    fn msub(x: FixpDbl, a: Self, b: B) -> FixpDbl;
    fn msub_div2(x: FixpDbl, a: Self, b: B) -> FixpDbl;
}

impl FMultAdd<FixpDbl> for FixpDbl {
    #[inline(always)]
    fn madd(x: i32, a: i32, b: i32) -> i32 {
        fixmadd_dd(x, a, b)
    }
    #[inline(always)]
    fn madd_div2(x: i32, a: i32, b: i32) -> i32 {
        fixmadddiv2_dd(x, a, b)
    }
    #[inline(always)]
    fn msub(x: i32, a: i32, b: i32) -> i32 {
        fixmsub_dd(x, a, b)
    }
    #[inline(always)]
    fn msub_div2(x: i32, a: i32, b: i32) -> i32 {
        fixmsubdiv2_dd(x, a, b)
    }
}

impl FMultAdd<FixpDbl> for FixpSgl {
    #[inline(always)]
    fn madd(x: i32, a: i16, b: i32) -> i32 {
        fixmadd_sd(x, a, b)
    }
    #[inline(always)]
    fn madd_div2(x: i32, a: i16, b: i32) -> i32 {
        fixmadddiv2_sd(x, a, b)
    }
    #[inline(always)]
    fn msub(x: i32, a: i16, b: i32) -> i32 {
        fixmsub_sd(x, a, b)
    }
    #[inline(always)]
    fn msub_div2(x: i32, a: i16, b: i32) -> i32 {
        fixmsubdiv2_sd(x, a, b)
    }
}

impl FMultAdd<FixpSgl> for FixpDbl {
    #[inline(always)]
    fn madd(x: i32, a: i32, b: i16) -> i32 {
        fixmadd_ds(x, a, b)
    }
    #[inline(always)]
    fn madd_div2(x: i32, a: i32, b: i16) -> i32 {
        fixmadddiv2_ds(x, a, b)
    }
    #[inline(always)]
    fn msub(x: i32, a: i32, b: i16) -> i32 {
        fixmsub_ds(x, a, b)
    }
    #[inline(always)]
    fn msub_div2(x: i32, a: i32, b: i16) -> i32 {
        fixmsubdiv2_ds(x, a, b)
    }
}

impl FMultAdd<FixpSgl> for FixpSgl {
    #[inline(always)]
    fn madd(x: i32, a: i16, b: i16) -> i32 {
        fixmadd_ss(x, a, b)
    }
    #[inline(always)]
    fn madd_div2(x: i32, a: i16, b: i16) -> i32 {
        fixmadddiv2_ss(x, a, b)
    }
    #[inline(always)]
    fn msub(x: i32, a: i16, b: i16) -> i32 {
        fixmsub_ss(x, a, b)
    }
    #[inline(always)]
    fn msub_div2(x: i32, a: i16, b: i16) -> i32 {
        fixmsubdiv2_ss(x, a, b)
    }
}

/// `fMultAdd(x, a, b)`: `2 * (x + a * b / 2)`
#[inline(always)]
pub fn f_mult_add<A: FMultAdd<B>, B>(x: FixpDbl, a: A, b: B) -> FixpDbl {
    A::madd(x, a, b)
}

/// `fMultAddDiv2(x, a, b)`: `x + a * b / 2`
#[inline(always)]
pub fn f_mult_add_div2<A: FMultAdd<B>, B>(x: FixpDbl, a: A, b: B) -> FixpDbl {
    A::madd_div2(x, a, b)
}

/// `fMultSub(x, a, b)`: `2 * (x - a * b / 2)`
#[inline(always)]
pub fn f_mult_sub<A: FMultAdd<B>, B>(x: FixpDbl, a: A, b: B) -> FixpDbl {
    A::msub(x, a, b)
}

/// `fMultSubDiv2(x, a, b)`: `x - a * b / 2`
#[inline(always)]
pub fn f_mult_sub_div2<A: FMultAdd<B>, B>(x: FixpDbl, a: A, b: B) -> FixpDbl {
    A::msub_div2(x, a, b)
}

/// Overloads of `fMultAddDiv2BitExact` and `fMultSubDiv2BitExact` (`FIXP_DBL` x `FIXP_DBL`,
/// `FIXP_SGL` x `FIXP_DBL`, `FIXP_DBL` x `FIXP_SGL`).
pub trait FMultAddBitExact<B>: Copy {
    fn madd_div2_bit_exact(x: FixpDbl, a: Self, b: B) -> FixpDbl;
    fn msub_div2_bit_exact(x: FixpDbl, a: Self, b: B) -> FixpDbl;
}

impl FMultAddBitExact<FixpDbl> for FixpDbl {
    #[inline(always)]
    fn madd_div2_bit_exact(x: i32, a: i32, b: i32) -> i32 {
        fixmadddiv2_bit_exact_dd(x, a, b)
    }
    #[inline(always)]
    fn msub_div2_bit_exact(x: i32, a: i32, b: i32) -> i32 {
        fixmsubdiv2_bit_exact_dd(x, a, b)
    }
}

impl FMultAddBitExact<FixpDbl> for FixpSgl {
    #[inline(always)]
    fn madd_div2_bit_exact(x: i32, a: i16, b: i32) -> i32 {
        fixmadddiv2_bit_exact_sd(x, a, b)
    }
    #[inline(always)]
    fn msub_div2_bit_exact(x: i32, a: i16, b: i32) -> i32 {
        fixmsubdiv2_bit_exact_sd(x, a, b)
    }
}

impl FMultAddBitExact<FixpSgl> for FixpDbl {
    #[inline(always)]
    fn madd_div2_bit_exact(x: i32, a: i32, b: i16) -> i32 {
        fixmadddiv2_bit_exact_ds(x, a, b)
    }
    #[inline(always)]
    fn msub_div2_bit_exact(x: i32, a: i32, b: i16) -> i32 {
        fixmsubdiv2_bit_exact_ds(x, a, b)
    }
}

/// `fMultAddDiv2BitExact(x, a, b)`
#[inline(always)]
pub fn f_mult_add_div2_bit_exact<A: FMultAddBitExact<B>, B>(x: FixpDbl, a: A, b: B) -> FixpDbl {
    A::madd_div2_bit_exact(x, a, b)
}

/// `fMultSubDiv2BitExact(x, a, b)`
#[inline(always)]
pub fn f_mult_sub_div2_bit_exact<A: FMultAddBitExact<B>, B>(x: FixpDbl, a: A, b: B) -> FixpDbl {
    A::msub_div2_bit_exact(x, a, b)
}

// ---------------------------------------------------------------------------------------------
// fMin, fMax
// ---------------------------------------------------------------------------------------------

/// `fMin(a, b)` (`FIXP_DBL`, `FIXP_SGL`, `UINT`, `UCHAR` overloads) / `fixMin(a, b)`
#[inline(always)]
pub fn f_min<T: PartialOrd>(a: T, b: T) -> T {
    fixmin(a, b)
}

/// `fMax(a, b)` (`FIXP_DBL`, `FIXP_SGL`, `UINT`, `UCHAR` overloads) / `fixMax(a, b)`
#[inline(always)]
pub fn f_max<T: PartialOrd>(a: T, b: T) -> T {
    fixmax(a, b)
}

/// `fixMin(a, b)`: `fMin(a, b)`
#[inline(always)]
pub fn fix_min<T: PartialOrd>(a: T, b: T) -> T {
    fixmin(a, b)
}

/// `fixMax(a, b)`: `fMax(a, b)`
#[inline(always)]
pub fn fix_max<T: PartialOrd>(a: T, b: T) -> T {
    fixmax(a, b)
}
