// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Optimized loops of the inverse modulations of libFDK/src/qmf.cpp.

//! SSE2 form of the NEON blocks of `neon.rs` (x86 hosts; SSE2 is part of the baseline of both
//! x86 targets): the iterations of the loop are independent (see `neon.rs`), four of them are
//! loaded, permuted and stored as vectors with the scalar operations per lane.

#[cfg(target_arch = "x86")]
use core::arch::x86::*;
#[cfg(target_arch = "x86_64")]
use core::arch::x86_64::*;

/// The four lanes in reverse order.
#[inline(always)]
unsafe fn rev(v: __m128i) -> __m128i {
    _mm_shuffle_epi32::<0b00_01_10_11>(v)
}

/// The wrapping negation of every lane.
#[inline(always)]
unsafe fn neg(v: __m128i) -> __m128i {
    _mm_sub_epi32(_mm_setzero_si128(), v)
}

#[inline(always)]
unsafe fn ld(p: *const i32) -> __m128i {
    _mm_loadu_si128(p as *const __m128i)
}

#[inline(always)]
unsafe fn st(p: *mut i32, v: __m128i) {
    _mm_storeu_si128(p as *mut __m128i, v)
}

/// The iterations `i = i0 .. i0 + 4` of the expansion loop of `qmfInverseModulationLP_even`
/// (`1 <= i0`, `i0 + 3 < m / 2`, `l = 2 m`), as `neon::lp_even_expand_block`.
///
/// # Safety
/// `re` and `im` point to `l` values each.
#[inline(always)]
pub(super) unsafe fn lp_even_expand_block(re: *mut i32, im: *mut i32, m: usize, i0: usize) {
    let l = 2 * m;
    let a = ld(re.add(i0)); // R[i]
    let b = ld(re.add(m - i0 - 3)); // R[m-i], reversed
    let c = ld(re.add(m + i0)); // R[m+i]
    let d = ld(re.add(l - i0 - 3)); // R[l-i], reversed
    st(im.add(m - i0 - 3), d);
    st(im.add(m + i0), neg(rev(d)));
    st(im.add(i0), c);
    st(im.add(l - i0 - 3), neg(rev(c)));
    st(re.add(m + i0), a);
    st(re.add(l - i0 - 3), b);
    st(re.add(i0), rev(b));
    st(re.add(m - i0 - 3), rev(a));
}

/// The iterations `i = i0 .. i0 + 4` of the final loop of `qmfInverseModulationLP_odd`
/// (`i0 + 3 < l / 2`): `p[i] = p[l-1-i]`, `p[2l-1-i] = -p[l+i]`, as
/// `neon::lp_odd_final_block`.
///
/// # Safety
/// `p` points to `2 l` values.
#[inline(always)]
pub(super) unsafe fn lp_odd_final_block(p: *mut i32, l: usize, i0: usize) {
    let a = ld(p.add(l - 4 - i0));
    let b = ld(p.add(l + i0));
    st(p.add(i0), rev(a));
    st(p.add(2 * l - 4 - i0), rev(neg(b)));
}
