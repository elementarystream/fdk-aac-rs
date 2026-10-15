// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// No C counterpart (an optimization of the port).

//! Run-time selection of the x86 code paths of the other crates (`fdk-aac-rs-sbrdec`,
//! `fdk-aac-rs-aacdec`, `fdk-aac-rs-arith`): AVX2 (the single detection of [`crate::simd`]), and
//! x86-64-v3 (AVX2 with BMI1, BMI2, LZCNT, FMA, MOVBE). The environment variable
//! `FDK_AAC_RS_NO_SIMD` (any value, read once) disables them; the portable code computes the same
//! values.

use std::sync::atomic::{AtomicU8, Ordering};

/// 0: not yet determined, else 1 + bit 0 (AVX2) + bit 1 (x86-64-v3).
static FEATURES: AtomicU8 = AtomicU8::new(0);

#[cold]
fn detect() -> u8 {
    let mut f = 0;
    if crate::simd::avx2() {
        f |= 1;
        if std::is_x86_feature_detected!("bmi1")
            && std::is_x86_feature_detected!("bmi2")
            && std::is_x86_feature_detected!("lzcnt")
            && std::is_x86_feature_detected!("fma")
            && std::is_x86_feature_detected!("movbe")
        {
            f |= 2;
        }
    }
    FEATURES.store(f + 1, Ordering::Relaxed);
    f + 1
}

#[inline]
fn features() -> u8 {
    match FEATURES.load(Ordering::Relaxed) {
        0 => detect() - 1,
        f => f - 1,
    }
}

/// `true` if the AVX2 code paths are used: the CPU has AVX2 and `FDK_AAC_RS_NO_SIMD` is not set.
#[inline]
pub fn avx2() -> bool {
    features() & 1 != 0
}

/// `true` if the x86-64-v3 code paths are used (AVX2, BMI1, BMI2, LZCNT, FMA, MOVBE, and
/// `FDK_AAC_RS_NO_SIMD` not set).
#[inline]
pub fn x86_64_v3() -> bool {
    features() & 2 != 0
}
