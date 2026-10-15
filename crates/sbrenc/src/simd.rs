// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// No counterpart in the reference (selection of the optimized x86 code paths).

//! Run-time selection of the x86 AVX2 code paths of the SBR encoder: the single detection of
//! `fdk_aac_rs_fdk::simd` (AVX2 present and `FDK_AAC_RS_NO_SIMD` not set).

/// `true` if the AVX2 code paths are used (see `fdk_aac_rs_fdk::simd::avx2`).
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[inline]
pub(crate) fn avx2() -> bool {
    fdk_aac_rs_fdk::simd::avx2()
}
