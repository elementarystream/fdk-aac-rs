// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libSYS/include/machine_type.h.

//! Port of `machine_type.h`: the integer types, the PCM sample format and a few build macros.
//!
//! The integer types map to Rust types directly (PORTING.md, "Porting rules"): `INT`, `LONG` =
//! `i32`; `UINT`, `ULONG` = `u32`; `SHORT` = `i16`; `USHORT` = `u16`; `SCHAR` = `i8`; `UCHAR` =
//! `u8`; `INT64` = `i64`; `UINT64` = `u64`.
//!
//! Not ported: `LINKSPEC_*`, `RESTRICT`, `WORD_ALIGNED`, `DWORD_ALIGNED`, `RAM_ALIGN`,
//! `FDK_FORCEINLINE`, `FDK_INLINE`, the `LNK_SECTION_*` macros and `FDK_FALLTHROUGH`, which do
//! not change any value. `FDK_ASSERT` is compiled out in the reference (`NDEBUG`); the port uses
//! `fdk_assert!` (PORTING.md, "Errors and assertions").

use crate::common_fix::{MAXVAL_SGL, MINVAL_SGL};

/// `SHORT_BITS`
pub const SHORT_BITS: i32 = 16;
/// `CHAR_BITS`
pub const CHAR_BITS: i32 = 8;

/// `INT_PCM`: the type of the input and output PCM samples (`SHORT`).
pub type IntPcm = i16;
/// `MAXVAL_PCM`: `MAXVAL_SGL` (an `int`)
pub const MAXVAL_PCM: i32 = MAXVAL_SGL;
/// `MINVAL_PCM`: `MINVAL_SGL` (an `int`)
pub const MINVAL_PCM: i32 = MINVAL_SGL;
/// `WAV_BITS`
pub const WAV_BITS: i32 = 16;
/// `SAMPLE_BITS`
pub const SAMPLE_BITS: i32 = 16;
/// `SAMPLE_MAX`: `(INT_PCM)(((ULONG)1 << (SAMPLE_BITS - 1)) - 1)`
pub const SAMPLE_MAX: IntPcm = ((1u32 << (SAMPLE_BITS - 1)) - 1) as IntPcm;
/// `SAMPLE_MIN`: `~SAMPLE_MAX`. In C the `~` promotes to `int`; the value is -32768.
pub const SAMPLE_MIN: i32 = !(SAMPLE_MAX as i32);

/// `ALIGNMENT_DEFAULT`: default alignment in bytes.
pub const ALIGNMENT_DEFAULT: usize = 8;
/// `ALIGNMENT_EXTRES`
pub const ALIGNMENT_EXTRES: usize = ALIGNMENT_DEFAULT;

/// `ALIGN_SIZE(a)`: `a` rounded up to a multiple of `ALIGNMENT_DEFAULT`.
#[inline(always)]
pub const fn align_size(a: usize) -> usize {
    a + ((ALIGNMENT_DEFAULT - (a & (ALIGNMENT_DEFAULT - 1))) & (ALIGNMENT_DEFAULT - 1))
}

/// `ALGN_SIZE_EXTRES(a)`: `a` rounded up to a multiple of `ALIGNMENT_EXTRES`.
#[inline(always)]
pub const fn algn_size_extres(a: usize) -> usize {
    a + ((ALIGNMENT_EXTRES - (a & (ALIGNMENT_EXTRES - 1))) & (ALIGNMENT_EXTRES - 1))
}
