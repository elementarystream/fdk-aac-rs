// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libSYS/include/genericStds.h, libSYS/src/genericStds.cpp.

//! Port of `genericStds.h` and `genericStds.cpp`: run-time support wrappers.
//!
//! What the library uses is ported as thin helpers on slices:
//!
//! | C++ | Port |
//! |---|---|
//! | `FDKmemcpy(dst, src, size)` | [`fdk_memcpy`] (`dst[..n].copy_from_slice(&src[..n])`) |
//! | `FDKmemmove(dst, src, size)` | [`fdk_memmove`]; overlapping moves within one slice: `copy_within` |
//! | `FDKmemclear(ptr, size)` | [`fdk_memclear`] |
//! | `FDKmemset(ptr, value, size)` | [`fdk_memset`] (bytes) |
//! | `FDKmemcmp(s1, s2, size)` | [`fdk_memcmp`] |
//! | `FDKstrlen`, `FDKstrcmp`, `FDKstrncmp` | [`fdk_strlen`], [`fdk_strcmp`], [`fdk_strncmp`] on NUL-terminated byte strings |
//! | `FDKmax`, `FDKmin` | [`fdk_max`], [`fdk_min`] |
//! | `IS_LITTLE_ENDIAN`, `TO_LITTLE_ENDIAN` | [`is_little_endian`], [`to_little_endian`] |
//!
//! The sizes of the C++ functions are in bytes; the helpers take element counts.
//!
//! Replaced by Rust language features and not ported:
//!
//! - `FDKcalloc`, `FDKmalloc`, `FDKfree`, `FDKaalloc`, `FDKafree` and their `_L` variants, the
//!   `C_ALLOC_MEM*`, `C_AALLOC_MEM*`, `H_ALLOC_MEM*` macros and the `C_*ALLOC_SCRATCH_*` /
//!   `C_AALLOC_STACK_*` macros: owned allocations (`Box`, `Vec`, arrays) and local buffers
//!   (PORTING.md, "Memory"). `calloc` clears the memory, so allocations that
//!   replace `FDKcalloc` must be zero-initialized.
//! - `FDKsprintf`: its only use (`LIB_VERSION_STRING`, `FDK_core.cpp`) is ported as
//!   [`crate::audio::lib_version_string`].
//! - `FDKprintf`, `FDKprintfErr`, `FDKfprintf`, `FDKgetchar`, `FDKprintDisclaimer`
//!   (PORTING.md, "What is not ported"), the string functions `FDKstrchr`, `FDKstrstr`,
//!   `FDKstrcpy`, `FDKstrncpy` and the file I/O functions (`FDKfopen` ... `FDKfread_EL`), which
//!   the library does not use.

/// `MEMORY_SECTION`
pub type MemorySection = i32;
pub const SECT_DATA_L1: MemorySection = 0x2000;
pub const SECT_DATA_L2: MemorySection = 0x2001;
pub const SECT_DATA_L1_A: MemorySection = 0x2002;
pub const SECT_DATA_L1_B: MemorySection = 0x2003;
pub const SECT_CONSTDATA_L1: MemorySection = 0x2004;
pub const SECT_DATA_EXTERN: MemorySection = 0x4000;
pub const SECT_CONSTDATA_EXTERN: MemorySection = 0x4001;

/// `M_PI` (`genericStds.h`, a `double`)
pub const M_PI: f64 = 3.14159265358979323846;

/// `FDK_MAX_OVERLAYS`: maximum number of memory overlays.
pub const FDK_MAX_OVERLAYS: i32 = 8;

/// `FDK_INT_MAX`
pub const FDK_INT_MAX: i32 = 0x7FFF_FFFF;
/// `FDK_INT_MIN`
pub const FDK_INT_MIN: i32 = 0x8000_0000u32 as i32;

/// `FDK_EXITCODE_OK`: successful termination.
pub const FDK_EXITCODE_OK: i32 = 0;
/// `FDK_EXITCODE_USAGE`: the command was used incorrectly.
pub const FDK_EXITCODE_USAGE: i32 = 64;
/// `FDK_EXITCODE_DATAERROR`: the input data was incorrect.
pub const FDK_EXITCODE_DATAERROR: i32 = 65;
/// `FDK_EXITCODE_NOINPUT`: an input file did not exist or was not readable.
pub const FDK_EXITCODE_NOINPUT: i32 = 66;
/// `FDK_EXITCODE_UNAVAILABLE`: a service is unavailable.
pub const FDK_EXITCODE_UNAVAILABLE: i32 = 69;
/// `FDK_EXITCODE_SOFTWARE`: an internal software error has been detected.
pub const FDK_EXITCODE_SOFTWARE: i32 = 70;
/// `FDK_EXITCODE_CANTCREATE`: an output file cannot be created.
pub const FDK_EXITCODE_CANTCREATE: i32 = 73;
/// `FDK_EXITCODE_IOERROR`: an error occurred while doing I/O on some file.
pub const FDK_EXITCODE_IOERROR: i32 = 74;

/// `FDKmemcpy(dst, src, n * sizeof(T))`: copies `n` elements (the areas must not overlap, which
/// Rust's borrow rules guarantee).
#[inline]
pub fn fdk_memcpy<T: Copy>(dst: &mut [T], src: &[T], n: usize) {
    dst[..n].copy_from_slice(&src[..n]);
}

/// `FDKmemmove(dst, src, n * sizeof(T))` between two different slices. A move within one
/// buffer is `buf.copy_within(src_range, dst_index)`.
#[inline]
pub fn fdk_memmove<T: Copy>(dst: &mut [T], src: &[T], n: usize) {
    dst[..n].copy_from_slice(&src[..n]);
}

/// `FDKmemclear(ptr, n * sizeof(T))`: sets `n` elements to zero (all bits clear).
#[inline]
pub fn fdk_memclear<T: Copy + Default>(dst: &mut [T], n: usize) {
    dst[..n].fill(T::default());
}

/// `FDKmemset(ptr, value, n)`: sets `n` bytes to `(unsigned char)value`.
#[inline]
pub fn fdk_memset(dst: &mut [u8], value: i32, n: usize) {
    dst[..n].fill(value as u8);
}

/// `FDKmemcmp(s1, s2, n)`: compares `n` bytes; the result is the difference of the first pair
/// of different bytes (as unsigned values), 0 if all are equal. C only guarantees the sign.
#[inline]
pub fn fdk_memcmp(s1: &[u8], s2: &[u8], n: usize) -> i32 {
    for (&a, &b) in s1[..n].iter().zip(&s2[..n]) {
        if a != b {
            return a as i32 - b as i32;
        }
    }
    0
}

/// `FDKstrlen(s)`: length of the NUL-terminated string `s` (the slice length if it has no NUL).
#[inline]
pub fn fdk_strlen(s: &[u8]) -> u32 {
    s.iter().position(|&c| c == 0).unwrap_or(s.len()) as u32
}

/// `FDKstrncmp(s1, s2, n)`: compares at most `n` characters of two NUL-terminated strings
/// (the end of a slice counts as NUL). Only the sign of the result is defined.
#[inline]
pub fn fdk_strncmp(s1: &[u8], s2: &[u8], n: usize) -> i32 {
    for i in 0..n {
        let a = s1.get(i).copied().unwrap_or(0);
        let b = s2.get(i).copied().unwrap_or(0);
        if a != b {
            return a as i32 - b as i32;
        }
        if a == 0 {
            break;
        }
    }
    0
}

/// `FDKstrcmp(s1, s2)`: compares two NUL-terminated strings (the end of a slice counts as NUL).
/// Only the sign of the result is defined.
#[inline]
pub fn fdk_strcmp(s1: &[u8], s2: &[u8]) -> i32 {
    fdk_strncmp(s1, s2, usize::MAX)
}

/// `FDKmax(a, b)`: `a > b ? a : b`
#[inline(always)]
pub fn fdk_max<T: PartialOrd>(a: T, b: T) -> T {
    if a > b {
        a
    } else {
        b
    }
}

/// `FDKmin(a, b)`: `a < b ? a : b`
#[inline(always)]
pub fn fdk_min<T: PartialOrd>(a: T, b: T) -> T {
    if a < b {
        a
    } else {
        b
    }
}

/// `IS_LITTLE_ENDIAN()`: 1 on little endian machines.
#[inline(always)]
pub const fn is_little_endian() -> i32 {
    cfg!(target_endian = "little") as i32
}

/// `TO_LITTLE_ENDIAN(val)`: byte swap on big endian machines.
#[inline(always)]
pub const fn to_little_endian(val: u32) -> u32 {
    val.to_le()
}
