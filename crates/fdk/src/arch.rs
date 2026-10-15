// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.

//! The architecture variant of the reference that the port reproduces.
//!
//! See PORTING.md, "Architecture variants". The variant is selected at build time by
//! `crates/fdk/build.rs` and exposed to the code as `cfg(fdk_arch = "...")`.

/// An architecture variant of the reference build.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arch {
    /// `__arm__` + `__ARM_ARCH_8__` (AArch64): `fMult` is `(a·b) >> 31`, integer math helpers,
    /// the ARM implementations of `dit_fft`, the `fft_N` kernels, `scaleValuesWithFactor` and
    /// `CBlock_ScaleSpectralData_func1`.
    Aarch64,
    /// `__x86__` on x86-64: `fMult` is `((a·b) >> 32) << 1`, `float` math helpers with SSE.
    X86,
    /// `__x86__` on 32-bit x86: as [`Arch::X86`], with the math helpers on the x87 unit.
    X86_32,
    /// Neither ARM nor x86 (for example RISC-V): the generic C code.
    Generic,
}

/// The variant selected for this build.
#[cfg(fdk_arch = "aarch64")]
pub const ARCH: Arch = Arch::Aarch64;
/// The variant selected for this build.
#[cfg(fdk_arch = "x86")]
pub const ARCH: Arch = Arch::X86;
/// The variant selected for this build.
#[cfg(fdk_arch = "x86_32")]
pub const ARCH: Arch = Arch::X86_32;
/// The variant selected for this build.
#[cfg(fdk_arch = "generic")]
pub const ARCH: Arch = Arch::Generic;
