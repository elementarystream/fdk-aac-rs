// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Optimized version of libFDK/src/fft_rad2.cpp (x86, x86_32 and generic variants).

//! AVX2 implementation of [`super::dit_fft_generic`] for `SineTable512` (`trigDataSize` 512),
//! selected at run time ([`avx2`]).
//!
//! In the x86 and generic variants `cplxMultDiv2()` (with `FIXP_SGL` coefficients) subtracts or
//! adds the individually truncated products `M(x, w) = (x * w) >> 16` (`fMultDiv2`), so every
//! output value of a radix-2 butterfly is `(x1 >> 1) +- (sa * M(x2, a) + sb * M(swap(x2), b))`,
//! where `swap(x2)` exchanges the real and imaginary parts and the coefficients `a`, `b` and the
//! signs `sa`, `sb` (+-1) of every value reproduce the four variants of the butterfly in the C
//! code (indices `k`, `mh/2 - k`, `mh/2 + k`, `mh - k` of the span `mh`, see `build`). The
//! trivial twiddle factors `k = 0` and `k = mh/2` use `x >> 1 == M(x, 32768)` and
//! `M(x, 0) == 0`.
//!
//! `M(x, w)` is computed exactly in 32 bit lanes as `(x >> 16) * w + (((x & 0xFFFF) * w) >> 16)`
//! (no intermediate result exceeds 31 bits for `|w| <= 32768`); a negative sign negates the
//! truncated product, and all sums wrap around like the C code.
//!
//! The bit reversal is fused into an out-of-place pass of the first two (radix-4) stages into a
//! buffer, which the first radix-2 stage reads. The radix-2 stages compute 4 butterflies (8
//! values of the interleaved real/imaginary data) at a time.

use crate::archdef::{stc, FixpStp};
use crate::common_fix::FixpDbl;
use crate::scramble::scramble;
use crate::tools_rom::SINE_TABLE_512;
#[cfg(target_arch = "x86")]
use core::arch::x86::*;
#[cfg(target_arch = "x86_64")]
use core::arch::x86_64::*;
use std::sync::OnceLock;

/// `true` if the AVX2 code paths are used (see [`crate::simd::avx2`]).
#[inline]
pub(crate) fn avx2() -> bool {
    crate::simd::avx2()
}

/// Smallest supported `ldn`.
pub(super) const MIN_LDN: i32 = 3;
/// Largest supported `ldn`.
pub(super) const MAX_LDN: i32 = 10;
const MAX_N: usize = 1 << MAX_LDN;

/// `(x * w) >> 16` per lane for 16 bit values `w` (`fixmuldiv2_DS`), computed exactly in 32 bit
/// lanes as `(x >> 16) * w + (((x & 0xFFFF) * w) >> 16)` (no intermediate value exceeds 31
/// bits).
#[inline(always)]
unsafe fn mul_div2(x: __m256i, w: __m256i) -> __m256i {
    let hi = _mm256_mullo_epi32(_mm256_srai_epi32::<16>(x), w);
    let lo = _mm256_and_si256(x, _mm256_set1_epi32(0xFFFF));
    _mm256_add_epi32(hi, _mm256_srai_epi32::<16>(_mm256_mullo_epi32(lo, w)))
}

/// Coefficients of the radix-2 stages `mh = 4, 8, ..., MAX_N / 2` for every value of the
/// interleaved data: stage `mh` uses the `2 * mh` entries from `2 * (mh - 4)`.
struct Tables {
    a: Box<[i32]>,
    b: Box<[i32]>,
    /// signs (1 or -1)
    sa: Box<[i32]>,
    sb: Box<[i32]>,
}

fn tables() -> &'static Tables {
    static TABLES: OnceLock<Tables> = OnceLock::new();
    TABLES.get_or_init(|| build(&SINE_TABLE_512, 512))
}

/// The coefficients for the sine table `trigdata` of size `trig_data_size`.
fn build(trigdata: &[FixpStp], trig_data_size: usize) -> Tables {
    let w = stc(0x5a82799a) as i32; // twiddle factor pi/4
    let len = 2 * (MAX_N - 4);
    let (mut a, mut b, mut sa, mut sb) =
        (Vec::with_capacity(len), Vec::with_capacity(len), Vec::new(), Vec::new());
    let mut mh = 4usize;
    // The C code halves trigstep (starting with trigDataSize) in every stage.
    let mut trigstep = trig_data_size >> 1;
    while mh < MAX_N {
        let q = mh / 4;
        let cs = |i: usize| {
            let t = trigdata[i * trigstep];
            (t.re as i32, t.im as i32)
        };
        for k in 0..mh {
            // vr = s1 M(re, a1) + s2 M(im, b1), vi = s3 M(re, a2) + s4 M(im, b2),
            // x1 = (ur + tr vr, ui + ti vi), x2 = (ur - tr vr, ui - ti vi); signs as +-1
            let [a1, s1, b1, s2, a2, s3, b2, s4, tr, ti] = if k == 0 {
                [32768, 1, 0, 1, 0, 1, 32768, 1, 1, 1]
            } else if k <= q {
                // (vi, vr) = cplxMultDiv2(x2.im, x2.re, c, s): vr = M(re, c) + M(im, s),
                // vi = M(im, c) - M(re, s)
                let (c, s) = if k == q { (w, w) } else { cs(k) };
                [c, 1, s, 1, s, -1, c, 1, 1, 1]
            } else if k < 2 * q {
                // index mh/2 - k': (vi, vr) = cplxMultDiv2(x2.re, x2.im, c, s):
                // vr = M(re, s) + M(im, c), vi = M(re, c) - M(im, s); x1 = (ur + vr, ui - vi)
                let (c, s) = cs(2 * q - k);
                [s, 1, c, 1, c, 1, s, -1, 1, -1]
            } else if k == 2 * q {
                // vr = x2.im >> 1, vi = x2.re >> 1; x1 = (ur + vr, ui - vi)
                [0, 1, 32768, 1, 32768, 1, 0, 1, 1, -1]
            } else if k <= 3 * q {
                // index mh/2 + k': (vr, vi) = cplxMultDiv2(x2.im, x2.re, c, s):
                // vr = M(im, c) - M(re, s), vi = M(im, s) + M(re, c); x1 = (ur + vr, ui - vi)
                let (c, s) = if k == 3 * q { (w, w) } else { cs(k - 2 * q) };
                [s, -1, c, 1, c, 1, s, 1, 1, -1]
            } else {
                // index mh - k': (vr, vi) = cplxMultDiv2(x2.re, x2.im, c, s):
                // vr = M(re, c) - M(im, s), vi = M(re, s) + M(im, c); x1 = (ur - vr, ui - vi)
                let (c, s) = cs(4 * q - k);
                [c, 1, s, -1, s, 1, c, 1, -1, -1]
            };
            // real part: x2 = re, swap(x2) = im
            a.push(a1);
            sa.push(tr * s1);
            b.push(b1);
            sb.push(tr * s2);
            // imaginary part: x2 = im, swap(x2) = re
            a.push(b2);
            sa.push(ti * s4);
            b.push(a2);
            sb.push(ti * s3);
        }
        mh <<= 1;
        trigstep >>= 1;
    }
    Tables {
        a: a.into_boxed_slice(),
        b: b.into_boxed_slice(),
        sa: sa.into_boxed_slice(),
        sb: sb.into_boxed_slice(),
    }
}

/// [`super::dit_fft_generic`] for `SineTable512` and `MIN_LDN <= ldn <= MAX_LDN`.
///
/// # Safety
///
/// AVX2 must be available.
#[target_feature(enable = "avx2")]
pub(super) unsafe fn dit_fft(x: &mut [FixpDbl], ldn: i32) {
    assert!((MIN_LDN..=MAX_LDN).contains(&ldn));
    let ldn = ldn as u32;
    let n = 1usize << ldn;
    let x = &mut x[..2 * n];
    let t = tables();
    let mut tmp = [core::mem::MaybeUninit::<i32>::uninit(); 2 * MAX_N];
    let tp = tmp.as_mut_ptr() as *mut i32;
    let mut mh = 4usize;
    if ldn >= 4 {
        // SAFETY: x holds 2 * n values and tmp at least 2 * n; the pass writes all of them.
        unsafe { first_pass(x.as_ptr(), tp, ldn) };
    } else {
        scramble(x, n as i32);
        // SAFETY: x holds 2 * n values.
        unsafe { radix4(x.as_mut_ptr(), n) };
    }
    let p = x.as_mut_ptr();
    // The first radix-2 stage reads the output of the first pass.
    let mut src = if ldn >= 4 { tp as *const i32 } else { p as *const i32 };
    while mh < n {
        let base = 2 * (mh - 4);
        // SAFETY: the stage reads and writes the values 2 * (g + k) .. 2 * (g + k + mh) + 8 with
        // g + k + mh + 4 <= n, and the coefficients base + 2 * k + 8 <= 2 * (MAX_N - 4).
        unsafe {
            stage(
                src,
                p,
                n,
                mh,
                t.a.as_ptr().add(base),
                t.b.as_ptr().add(base),
                t.sa.as_ptr().add(base),
                t.sb.as_ptr().add(base),
            )
        };
        src = p;
        mh <<= 1;
    }
}

/// Bit reversal of 8 bit values.
static BITREV8: [u8; 256] = {
    let mut t = [0u8; 256];
    let mut i = 0;
    while i < 256 {
        t[i] = (i as u8).reverse_bits();
        i += 1;
    }
    t
};

/// Bit reversal permutation and the radix-4 butterflies of the first two stages (see
/// `radix4_first_stages()`) from `x` into `y` (`2 * 2^ldn` values each, `ldn >= 4`): the
/// output group `g` (4 complex values) reads the inputs `r`, `r + n/2`, `r + n/4`,
/// `r + 3n/4` with `r = bitrev(g)`, so 4 consecutive `r` are loaded as 4 vectors, the
/// butterflies computed on the interleaved values, and the results transposed into the 4
/// output groups `bitrev(r + l) = bitrev(r / 4) + rev2(l) << (ldn - 4)`.
#[inline]
#[target_feature(enable = "avx2")]
unsafe fn first_pass(x: *const i32, y: *mut i32, ldn: u32) {
    let q = 1usize << (ldn - 2);
    let kb = ldn - 4;
    let sign = _mm256_setr_epi32(1, -1, 1, -1, 1, -1, 1, -1);
    for u in 0..q / 4 {
        let r = 4 * u;
        // SAFETY: r + 3q + 4 <= n; the groups are below n / 4.
        unsafe {
            let ld = |k: usize| _mm256_loadu_si256(x.add(2 * k) as *const __m256i);
            let (c0, c1, c2, c3) = (ld(r), ld(r + 2 * q), ld(r + q), ld(r + 3 * q));
            let s = _mm256_srai_epi32::<1>(_mm256_add_epi32(c0, c1));
            let t = _mm256_srai_epi32::<1>(_mm256_add_epi32(c2, c3));
            let y0 = _mm256_add_epi32(s, t);
            let y2 = _mm256_sub_epi32(s, t);
            let s = _mm256_sub_epi32(s, c1);
            let t = _mm256_sub_epi32(t, c3);
            // (t.im, -t.re)
            let ts = _mm256_sign_epi32(_mm256_shuffle_epi32::<0b10_11_00_01>(t), sign);
            let y1 = _mm256_add_epi32(s, ts);
            let y3 = _mm256_sub_epi32(s, ts);
            // transpose the 4 x 4 complex values
            let t0 = _mm256_unpacklo_epi64(y0, y1);
            let t1 = _mm256_unpackhi_epi64(y0, y1);
            let t2 = _mm256_unpacklo_epi64(y2, y3);
            let t3 = _mm256_unpackhi_epi64(y2, y3);
            let g = (BITREV8[u] as usize) >> (8 - kb);
            let st = |g: usize, v: __m256i| _mm256_storeu_si256(y.add(8 * g) as *mut __m256i, v);
            st(g, _mm256_permute2x128_si256::<0x20>(t0, t2));
            st(g + (2 << kb), _mm256_permute2x128_si256::<0x20>(t1, t3));
            st(g + (1 << kb), _mm256_permute2x128_si256::<0x31>(t0, t2));
            st(g + (3 << kb), _mm256_permute2x128_si256::<0x31>(t1, t3));
        }
    }
}

/// The combined radix-4 butterflies of the first two stages (see `radix4_first_stages()`),
/// one per register `(r0 i0 r1 i1 r2 i2 r3 i3)`: the sums `a = (x0 + x1) >> 1` and the
/// differences `b = a - x1` of the complex pairs `(x0, x1)` give `t = (a0 b0 | a1 b1)` (complex
/// values `a00 a20 b00 b20 | a10 a30 b10 b30` in the C++ code), the output is
/// `(lo + h | lo - h)` with `lo = (a00 a20 b00 b20)` and `h = (a10 a30 b30 -b10)`.
#[inline]
#[target_feature(enable = "avx2")]
unsafe fn radix4(x: *mut i32, n: usize) {
    let sign = _mm256_setr_epi32(1, 1, 1, -1, -1, -1, -1, 1);
    let idx_h = _mm256_setr_epi32(4, 5, 7, 6, 4, 5, 7, 6);
    for k in 0..n / 4 {
        // SAFETY: the values 8k .. 8k + 8 are within the 2n values.
        unsafe {
            let p = x.add(8 * k) as *mut __m256i;
            let v = _mm256_loadu_si256(p);
            let vs = _mm256_shuffle_epi32::<0b01_00_11_10>(v);
            let a = _mm256_srai_epi32::<1>(_mm256_add_epi32(v, vs));
            let t = _mm256_blend_epi32::<0b1100_1100>(a, _mm256_sub_epi32(a, v));
            let lo = _mm256_permute4x64_epi64::<0b01_00_01_00>(t);
            let h = _mm256_sign_epi32(_mm256_permutevar8x32_epi32(t, idx_h), sign);
            _mm256_storeu_si256(p, _mm256_add_epi32(lo, h));
        }
    }
}

/// `v` negated where `sign` is -1.
#[inline]
#[target_feature(enable = "avx2")]
unsafe fn cneg(v: __m256i, sign: __m256i) -> __m256i {
    _mm256_sign_epi32(v, sign)
}

/// One radix-2 stage with the span `mh >= 4` (complex values) from `src` to `dst` (the
/// same buffer, or different ones).
#[allow(clippy::too_many_arguments)]
#[inline]
#[target_feature(enable = "avx2")]
unsafe fn stage(
    src: *const i32,
    dst: *mut i32,
    n: usize,
    mh: usize,
    a: *const i32,
    b: *const i32,
    sa: *const i32,
    sb: *const i32,
) {
    let mut k = 0usize;
    while k < mh {
        // SAFETY: see `dit_fft`.
        unsafe {
            let ca = _mm256_loadu_si256(a.add(2 * k) as *const __m256i);
            let cb = _mm256_loadu_si256(b.add(2 * k) as *const __m256i);
            let ma = _mm256_loadu_si256(sa.add(2 * k) as *const __m256i);
            let mb = _mm256_loadu_si256(sb.add(2 * k) as *const __m256i);
            let mut g = k;
            while g < n {
                let p1 = dst.add(2 * g) as *mut __m256i;
                let p2 = dst.add(2 * (g + mh)) as *mut __m256i;
                let x1 = _mm256_loadu_si256(src.add(2 * g) as *const __m256i);
                let x2 = _mm256_loadu_si256(src.add(2 * (g + mh)) as *const __m256i);
                let x2s = _mm256_shuffle_epi32::<0b10_11_00_01>(x2);
                let v = _mm256_add_epi32(cneg(mul_div2(x2, ca), ma), cneg(mul_div2(x2s, cb), mb));
                let u = _mm256_srai_epi32::<1>(x1);
                _mm256_storeu_si256(p1, _mm256_add_epi32(u, v));
                _mm256_storeu_si256(p2, _mm256_sub_epi32(u, v));
                g += 2 * mh;
            }
        }
        k += 4;
    }
}
