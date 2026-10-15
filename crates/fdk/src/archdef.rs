// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libFDK/include/FDK_archdef.h.

//! Port of `FDK_archdef.h`.
//!
//! `FDK_archdef.h` derives the architecture macros: `__x86__` for x86 targets, `__arm__` and
//! `__ARM_ARCH_8__` for `__aarch64__`. They select the variant of the primitives and of a few
//! functions; the port selects them with [`crate::ARCH`] (PORTING.md, "Architecture
//! variants").
//!
//! Both reference architectures define `ARCH_PREFER_MULT_32x16`, `SINETABLE_16BIT`,
//! `POW2COEFF_16BIT`, `LDCOEFF_16BIT` and `WINDOWTABLE_16BIT`, so the table types below are the
//! same on every variant.

use crate::common_fix::{fx_dbl2fx_sgl, fx_dbl2fxconst_sgl, FixpDbl, FixpSgl, FixpSpk};

/// `FIXP_STB` (`SINETABLE_16BIT`): sine table entries
pub type FixpStb = FixpSgl;
/// `FIXP_STP` (`SINETABLE_16BIT`): packed sine table entries
pub type FixpStp = FixpSpk;
/// `FIXP_WTB` (`WINDOWTABLE_16BIT`): window table entries
pub type FixpWtb = FixpSgl;
/// `FIXP_WTP` (`WINDOWTABLE_16BIT`): packed window table entries
pub type FixpWtp = FixpSpk;

/// `STC(a)`: `FX_DBL2FXCONST_SGL(a)`
#[inline(always)]
pub const fn stc(a: FixpDbl) -> FixpStb {
    fx_dbl2fxconst_sgl(a)
}

/// `STCP(cos, sin)`
#[inline(always)]
pub const fn stcp(cos: FixpDbl, sin: FixpDbl) -> FixpStp {
    FixpSpk { re: stc(cos), im: stc(sin) }
}

/// `WTC(a)`: `FX_DBL2FXCONST_SGL(a)`
#[inline(always)]
pub const fn wtc(a: FixpDbl) -> FixpWtb {
    fx_dbl2fxconst_sgl(a)
}

/// `WTCP(a, b)`
#[inline(always)]
pub const fn wtcp(a: FixpDbl, b: FixpDbl) -> FixpWtp {
    FixpSpk { re: wtc(a), im: wtc(b) }
}

/// `FX_DBL2FX_WTB(x)`: `FX_DBL2FX_SGL(x)`
#[inline(always)]
pub const fn fx_dbl2fx_wtb(x: FixpDbl) -> FixpWtb {
    fx_dbl2fx_sgl(x)
}
