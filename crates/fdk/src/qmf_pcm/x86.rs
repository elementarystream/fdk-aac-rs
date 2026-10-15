// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Optimized synthesis prototype filters of libFDK/include/qmf_pcm.h (x86 targets).

//! AVX2 kernel of the QMF synthesis prototype filters (the same arithmetic on every variant),
//! selected at run time ([`crate::simd::avx2`]).
//!
//! The filter of a channel updates its 9 states `s` with the 10 products
//! `P[k] = fMultDiv2(c[k], x[k])` (`x[k]` the real input for even `k`, the imaginary one for odd
//! `k`): the output is `s[0] + P[0]`, then `s[k] = s[k + 1] + P[k + 1]` for `k < 8` and
//! `s[8] = P[9]` (see `super::syn_channel`). The kernel computes `P[1..9]` in the 8 lanes of a
//! vector against the states `s[1..9]`, and `P[0]`, `P[9]` scalar.
//!
//! `fMultDiv2(FIXP_SGL c, FIXP_DBL x)` (`fixmuldiv2_SD`) is `(c * x) >> 16`, computed exactly in
//! 32-bit lanes as `(x >> 16) * c + (((x & 0xFFFF) * c) >> 16)` (no intermediate value exceeds
//! 31 bits). The sums wrap like the scalar code.

#[cfg(target_arch = "x86")]
use core::arch::x86::*;
#[cfg(target_arch = "x86_64")]
use core::arch::x86_64::*;

/// The coefficients of a channel in the order of the products: `[c[1..9], c[0], c[9]]`, with
/// `c` = `fltm[0], flt[4], fltm[1], flt[3], fltm[2], flt[2], fltm[3], flt[1], fltm[4], flt[0]`
/// (symmetric) or `fltm[4], flt[4], fltm[3], flt[3], fltm[2], flt[2], fltm[1], flt[1], fltm[0],
/// flt[0]` (non-symmetric), as in `super::syn_channel`.
pub(super) fn syn_coefs<const NONSYM: bool>(
    p_filter: &[i16],
    flt0: usize,
    fltm0: usize,
    step: usize,
    nch: usize,
) -> Box<[i32]> {
    let mut out = vec![0i32; 10 * nch];
    for it in 0..nch {
        let flt = &p_filter[flt0 + it * step..][..5];
        let fltm = if NONSYM {
            &p_filter[fltm0 + it * step..][..5]
        } else {
            &p_filter[fltm0 - it * step..][..5]
        };
        let c: [i16; 10] = if NONSYM {
            [fltm[4], flt[4], fltm[3], flt[3], fltm[2], flt[2], fltm[1], flt[1], fltm[0], flt[0]]
        } else {
            [fltm[0], flt[4], fltm[1], flt[3], fltm[2], flt[2], fltm[3], flt[1], fltm[4], flt[0]]
        };
        let o = &mut out[10 * it..10 * it + 10];
        for k in 0..8 {
            o[k] = c[k + 1] as i32;
        }
        o[8] = c[0] as i32;
        o[9] = c[9] as i32;
    }
    out.into_boxed_slice()
}

/// `(x * c) >> 16` per lane for 16-bit `c` (see the module documentation).
#[inline(always)]
unsafe fn mul_div2(x: __m256i, c: __m256i) -> __m256i {
    let hi = _mm256_mullo_epi32(_mm256_srai_epi32::<16>(x), c);
    let lo = _mm256_and_si256(x, _mm256_set1_epi32(0xFFFF));
    _mm256_add_epi32(hi, _mm256_srai_epi32::<16>(_mm256_mullo_epi32(lo, c)))
}

/// `(x * c) >> 16` for a 16-bit `c`.
#[inline(always)]
fn md2(c: i32, x: i32) -> i32 {
    ((c as i64 * x as i64) >> 16) as i32
}

/// The filters of all `nch = are.len()` channels: channel `it` (states `s[9 it..9 it + 9]`,
/// coefficients `c[10 it..10 it + 10]` of [`syn_coefs`], inputs `real[j]`, `imag[j]` with
/// `j = nch - 1 - it`) writes its output before the PCM formatting to `are[it]`.
///
/// # Safety
///
/// AVX2 must be available.
#[target_feature(enable = "avx2")]
pub(super) unsafe fn syn_channels(
    s: &mut [i32],
    c: &[i32],
    real: &[i32],
    imag: &[i32],
    are: &mut [i32],
) {
    let nch = are.len();
    let (s, c, real, imag) = (&mut s[..9 * nch], &c[..10 * nch], &real[..nch], &imag[..nch]);
    for (it, a) in are.iter_mut().enumerate() {
        let j = nch - 1 - it;
        let (re, im) = (real[j], imag[j]);
        let cc = &c[10 * it..10 * it + 10];
        let st = &mut s[9 * it..9 * it + 9];
        // SAFETY: AVX2 is available; `cc` holds 10 values (8 loaded), `st` 9 (8 loaded from
        // index 1 and 8 stored from index 0).
        unsafe {
            // lanes k = 1..9: the imaginary input for odd k, the real one for even k
            let x = _mm256_set1_epi64x(((re as u32 as u64) << 32 | im as u32 as u64) as i64);
            let p = mul_div2(x, _mm256_loadu_si256(cc.as_ptr() as *const __m256i));
            let u = _mm256_loadu_si256(st.as_ptr().add(1) as *const __m256i);
            *a = st[0].wrapping_add(md2(cc[8], re));
            _mm256_storeu_si256(st.as_mut_ptr() as *mut __m256i, _mm256_add_epi32(u, p));
        }
        st[8] = md2(cc[9], im);
    }
}
