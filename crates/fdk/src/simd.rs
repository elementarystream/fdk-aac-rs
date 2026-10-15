// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.

//! Run-time selection of the x86 SIMD code paths (PORTING.md, "Optimization rules").
//!
//! The x86-64 and i686 targets guarantee SSE2 only, so the AVX2 code paths are selected at run
//! time. They compute exactly the same values as the portable code. The environment variable
//! `FDK_AAC_RS_NO_SIMD` (any value, read once) disables them, e.g. for comparisons and to test
//! the portable code on machines with AVX2.

/// `true` if the AVX2 code paths are used: an x86 or x86-64 CPU with AVX2, and the environment
/// variable `FDK_AAC_RS_NO_SIMD` is not set. Always `false` on other targets.
#[inline]
pub fn avx2() -> bool {
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        use std::sync::atomic::{AtomicU8, Ordering};
        // 0: not yet determined, 1: no, 2: yes
        static AVX2: AtomicU8 = AtomicU8::new(0);
        match AVX2.load(Ordering::Relaxed) {
            2 => true,
            1 => false,
            _ => {
                let on = std::is_x86_feature_detected!("avx2")
                    && std::env::var_os("FDK_AAC_RS_NO_SIMD").is_none();
                AVX2.store(if on { 2 } else { 1 }, Ordering::Relaxed);
                on
            }
        }
    }
    #[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
    {
        false
    }
}

/// Defines a function whose body is compiled twice on x86 and x86-64 targets: for the baseline
/// target (SSE2) and with AVX2 enabled, selected at run time with [`avx2()`]. The compiler
/// vectorizes the loops of the AVX2 clone with 256-bit registers and the AVX2 instructions (e.g.
/// `vpmuldq` for the 64-bit products of `fMultDiv2()`), which compute the same values: both
/// clones are the same Rust code. On other targets the function is compiled once.
///
/// The parameters must be plain identifiers (no patterns, no `self`); generic parameters are
/// not supported.
#[macro_export]
macro_rules! avx2_clone {
    (
        $(#[$attr:meta])*
        $vis:vis fn $name:ident($($arg:ident: $ty:ty),* $(,)?) $(-> $ret:ty)? $body:block
    ) => {
        $(#[$attr])*
        #[allow(clippy::too_many_arguments)]
        $vis fn $name($($arg: $ty),*) $(-> $ret)? {
            #[inline(always)]
            #[allow(clippy::too_many_arguments)]
            fn imp($($arg: $ty),*) $(-> $ret)? $body
            #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
            if $crate::simd::avx2() {
                #[target_feature(enable = "avx2")]
                #[allow(clippy::too_many_arguments)]
                unsafe fn avx2($($arg: $ty),*) $(-> $ret)? {
                    imp($($arg),*)
                }
                // SAFETY: the CPU supports AVX2.
                return unsafe { avx2($($arg),*) };
            }
            imp($($arg),*)
        }
    };
}
