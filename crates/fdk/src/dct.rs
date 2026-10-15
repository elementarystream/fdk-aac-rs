// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libFDK/include/dct.h, libFDK/src/dct.cpp.

//! Port of `dct.h`: DCT-II, DCT-III, DST-III, DCT-IV and DST-IV, computed with a complex FFT of
//! half the length and pre/post twiddling.
//!
//! All transforms work in place on the first `L` values of `p_dat`; the exponent `p_dat_e` is
//! updated by the scaling of the transform. The `.ii` files of the `aarch64` and `x86` builds
//! are identical for `dct.cpp` apart from `FDK_ASSERT`; the variants differ only through the
//! primitives (`cplxMultDiv2`, `cplxMult`, `fMult`) and the FFT ([`crate::fft`]).
//!
//! `dct_getTables` supports four length rasters (radix 2, 10 ms, 3/4 of radix 2
//! and 5/16 of radix 2), so every length `L = 2 * M` works for which [`fft`] supports `M` and the
//! window slope table of `L` exists.
//!
//! Deviation: for a length without tables (`dct_getTables` returns `NULL`) the reference has no
//! defined result; the port leaves the data unchanged.
//!
//! The additions and negations wrap (signed overflow is undefined in C); the `cref` tests with
//! full-scale input give the same values as both reference builds.

use crate::archdef::{wtc, FixpStp, FixpWtp};
use crate::common_fix::*;
use crate::fft::fft;
use crate::tools_rom::{
    SINE_TABLE_1024, SINE_TABLE_384, SINE_TABLE_480, SINE_TABLE_80, WINDOW_SLOPES,
};

/// `windowSlopes[0][raster][idx]`, with the C indexing beyond a row continuing into the next
/// rows; `None` outside of the table.
fn window_slope(raster: i32, idx: i32) -> Option<&'static [FixpWtp]> {
    let flat = WINDOW_SLOPES.as_flattened().as_flattened();
    let i = raster * 9 + idx;
    if (0..flat.len() as i32).contains(&i) {
        flat[i as usize]
    } else {
        None
    }
}

/// `dct_getTables(&twiddle, &sin_twiddle, &sin_step, length)`: returns
/// `(twiddle, sin_twiddle, sin_step)` for the transform length (`None`, `None`, 0 for an
/// unsupported length).
pub fn dct_get_tables(
    length: i32,
) -> (Option<&'static [FixpWtp]>, Option<&'static [FixpStp]>, i32) {
    // Get ld2 of length - 2 + 1
    //   -2: because first table entry is window of size 4
    //   +1: because we already include +1 because of ceil(log2(length))
    let ld2_length = DFRACT_BITS - 1 - f_normz(length) - 1;

    // Extract sort of "eigenvalue" (the 4 left most bits) of length.
    // The shift amount is negative for lengths below 4 (undefined in C, never used); the port
    // takes it modulo 32 like the `asr`/`sar` instructions.
    let (sin_twiddle, sin_step, twiddle): (Option<&'static [FixpStp]>, i32, _) =
        match length.wrapping_shr((ld2_length - 1) as u32) {
            0x4 => {
                // radix 2
                (Some(&SINE_TABLE_1024), 1i32.wrapping_shl((10 - ld2_length) as u32), {
                    window_slope(0, ld2_length - 1)
                })
            }
            0x7 => {
                // 10 ms
                (Some(&SINE_TABLE_480), 1i32.wrapping_shl((8 - ld2_length) as u32), {
                    window_slope(1, ld2_length)
                })
            }
            0x6 => {
                // 3/4 of radix 2
                (Some(&SINE_TABLE_384), 1i32.wrapping_shl((8 - ld2_length) as u32), {
                    window_slope(2, ld2_length)
                })
            }
            0x5 => {
                // 5/16 of radix 2
                (Some(&SINE_TABLE_80), 1i32.wrapping_shl((6 - ld2_length) as u32), {
                    window_slope(3, ld2_length)
                })
            }
            _ => (None, 0, None),
        };

    fdk_assert!(sin_step > 0);

    (twiddle, sin_twiddle, sin_step)
}

/// `dct_III(pDat, tmp, L, pDat_e)`: DCT-III of length `L`.
///
/// Note that the factor 0.5 for the sum term `x[0]` is 1.0 instead of 0.5. A factor of
/// `sqrt(2/N)` is not applied. `tmp` is a work buffer of at least `L` values.
///
/// The same as the straight port [`dct_iii_ref`]: the iterations of its loops are independent
/// (they read `pDat` and write `tmp`, or the reverse), so on `aarch64` targets four of them run
/// at a time in NEON for the `aarch64` variant (see `neon::dct_iii_block`).
pub fn dct_iii(p_dat: &mut [FixpDbl], tmp: &mut [FixpDbl], l: i32, p_dat_e: &mut i32) {
    #[cfg(target_arch = "aarch64")]
    if NEON && l == 32 && p_dat.len() >= 32 && tmp.len() >= 32 {
        return dct_iii_neon_32(p_dat, tmp, p_dat_e);
    }
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    if !ARCH_ARM && l == 32 && p_dat.len() >= 32 && tmp.len() >= 32 && crate::simd::avx2() {
        // SAFETY: the CPU has AVX2.
        if unsafe { dct_iii_x86_32(p_dat, tmp, p_dat_e) } {
            return;
        }
    }
    let m = l >> 1;

    fdk_assert!(l % 4 == 0);
    let (_, sin_twiddle, inc) = dct_get_tables(l);
    let Some(sin_twiddle) = sin_twiddle else { return };
    let inc = inc >> 1;

    let (l, m, inc) = (l as usize, m as usize, inc as usize);
    let (p_dat, tmp) = (&mut p_dat[..l], &mut tmp[..l]);
    let tw = |k: usize| sin_twiddle[k];

    // This loop performs multiplication for index i (i*inc)
    let half = m >> 1;
    #[allow(unused_mut)]
    let mut i = 1usize;
    #[cfg(target_arch = "aarch64")]
    if NEON && half >= 5 {
        // Four iterations at a time, either branch per lane (see `neon::dct_iii_block`); the
        // last block may overlap the previous one: the iterations read p_dat and write tmp, so
        // computing one again writes the same values. The twiddle indices are at most m * inc.
        assert!(sin_twiddle.len() > m * inc);
        let split = half.div_ceil(2); // the first i with 2 * i >= m / 2
        while i < half {
            let i0 = i.min(half - 4);
            // SAFETY: the iterations read p_dat[i], p_dat[l-i], p_dat[m+i], p_dat[m-i] and
            // write tmp[2i..2i+2], tmp[2(m-i)..2(m-i)+2] for 1 <= i < m/2, inside the l values;
            // the twiddle indices are checked above.
            unsafe {
                let (x, t, w) = (p_dat.as_ptr(), tmp.as_mut_ptr(), sin_twiddle.as_ptr());
                neon::dct_iii_block(x, t, l, m, i0, w, inc, split);
            }
            i = i0 + 4;
        }
    }
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    if !ARCH_ARM && half >= 5 {
        // As above, with the SSE2 blocks of the other variants (`sse2::dct_iii_block`) and the
        // twiddles of the configuration in a table (`sse2::twiddle_table`).
        assert!(sin_twiddle.len() > m * inc);
        let split = half.div_ceil(2); // the first i with 2 * i >= m / 2
        if let Some(tab) = sse2::twiddle_table(sin_twiddle, m, inc, split) {
            while i < half {
                let i0 = i.min(half - 4);
                // SAFETY: as above; the table holds 3 * (m / 2) words.
                unsafe {
                    let (x, t) = (p_dat.as_ptr(), tmp.as_mut_ptr());
                    sse2::dct_iii_block(x, t, l, m, i0, tab.as_ptr());
                }
                i = i0 + 4;
            }
        }
    }
    while i < half {
        dct_iii_iter(p_dat, tmp, l, m, inc, i, dct_twiddle_index(m, inc, i), sin_twiddle);
        i += 1;
    }

    let xr = f_mult_div2(p_dat[m], tw(m * inc).re); // cos((PI/(2*L))*M);
    tmp[0] = ((p_dat[0] >> 1).wrapping_add(xr)) >> 1;
    tmp[1] = ((p_dat[0] >> 1).wrapping_sub(xr)) >> 1;

    let w = tw(m * inc / 2);
    let (accu2, accu1) = cplx_mult_div2(p_dat[l - (m / 2)], p_dat[m / 2], w.re, w.im);
    tmp[m] = accu1 >> 1;
    tmp[m + 1] = accu2 >> 1;

    // dit_fft expects 1 bit scaled input values
    fft(m as i32, tmp, p_dat_e);

    #[allow(unused_mut)]
    let mut k = 0usize;
    #[cfg(target_arch = "aarch64")]
    while k + 4 <= m >> 1 {
        // SAFETY: reads tmp[2k..2k+8] and tmp[l-8-2k..l-2k], writes p_dat[4k..4k+16], inside
        // the l values for k + 4 <= m / 2.
        unsafe { neon::dct_iii_post_block(p_dat.as_mut_ptr(), tmp.as_ptr(), l, k) };
        k += 4;
    }
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    while !ARCH_ARM && k + 4 <= m >> 1 {
        // SAFETY: as above.
        unsafe { sse2::dct_iii_post_block(p_dat.as_mut_ptr(), tmp.as_ptr(), l, k) };
        k += 4;
    }
    while k < m >> 1 {
        p_dat[4 * k] = tmp[2 * k];
        p_dat[4 * k + 1] = tmp[l - 1 - 2 * k];
        p_dat[4 * k + 2] = tmp[2 * k + 1];
        p_dat[4 * k + 3] = tmp[l - 2 - 2 * k];
        k += 1;
    }

    *p_dat_e += 2;
}

/// The value of `index` in the iteration `i` (`1 <= i < m / 2`) of the loops of `dct_III` and
/// `dct_II`: `4 inc i` for `2 i < m / 2`, else `4 inc (m / 2 - i)`.
///
/// The C code starts with `4 inc` and adds `4 inc` after the iterations with `2 i < m / 2 - 1`,
/// keeps it after `2 i == m / 2 - 1` and subtracts `4 inc` after `2 i >= m / 2`; for
/// `2 i < m / 2` all earlier iterations added, which gives `4 inc i`; for `2 i >= m / 2`
/// (`i >= ceil(m / 4)`) the `floor(m / 4) - 1` additions and the `i - ceil(m / 4)`
/// subtractions give `4 inc (m / 2 - i)` (`dct_twiddle_indices` in the tests).
#[inline(always)]
fn dct_twiddle_index(m: usize, inc: usize, i: usize) -> usize {
    4 * inc * if 2 * i < m / 2 { i } else { m / 2 - i }
}

/// The value of `index` in the iterations `i = 1 .. m / 2` of the loops of `dct_III` and
/// `dct_II`, computed like the C code (`[i]`; `[0]` unused).
#[cfg(test)]
fn dct_twiddle_indices(m: usize, inc: usize) -> [usize; 512] {
    let mut out = [0usize; 512];
    let mut index = 4 * inc;
    let mut i = 1usize;
    while i < m >> 1 {
        out[i] = index;
        // Create index helper variables for (4*i)*inc indexed equivalent values of short
        // tables.
        if 2 * i < ((m / 2).wrapping_sub(1)) {
            index += 4 * inc;
        } else if 2 * i >= (m / 2) {
            index = index.wrapping_sub(4 * inc);
        }
        i += 1;
    }
    out
}

/// The iteration `i` of the first loop of [`dct_iii_ref`] (`index`: its twiddle index); reads
/// `p_dat`, writes `tmp[2i]`, `tmp[2i+1]`, `tmp[2(m-i)]`, `tmp[2(m-i)+1]`.
#[allow(clippy::too_many_arguments)]
#[inline(always)]
fn dct_iii_iter(
    p_dat: &[FixpDbl],
    tmp: &mut [FixpDbl],
    l: usize,
    m: usize,
    inc: usize,
    i: usize,
    index: usize,
    sin_twiddle: &[FixpStp],
) {
    let tw = |k: usize| sin_twiddle[k];
    let (accu2, accu1) = cplx_mult_div2(p_dat[l - i], p_dat[i], tw(i * inc).re, tw(i * inc).im);
    let w = tw((m - i) * inc);
    let (mut accu4, mut accu3) = cplx_mult_div2(p_dat[m + i], p_dat[m - i], w.re, w.im);
    accu3 >>= 1;
    accu4 >>= 1;

    let w = tw(index);
    let (accu6, accu5);
    if 2 * i < (m / 2) {
        (accu6, accu5) = cplx_mult_div2(
            accu3.wrapping_sub(accu1 >> 1),
            (accu2 >> 1).wrapping_add(accu4),
            w.re,
            w.im,
        );
    } else {
        let a6;
        (a6, accu5) = cplx_mult_div2(
            (accu2 >> 1).wrapping_add(accu4),
            accu3.wrapping_sub(accu1 >> 1),
            w.re,
            w.im,
        );
        accu6 = a6.wrapping_neg();
    }
    let (p_tmp_0, p_tmp_1) = (2 * i, 2 * (m - i));
    let mut xr = (accu1 >> 1).wrapping_add(accu3);
    tmp[p_tmp_0] = (xr >> 1).wrapping_sub(accu5);
    tmp[p_tmp_1] = (xr >> 1).wrapping_add(accu5);

    xr = (accu2 >> 1).wrapping_sub(accu4);
    tmp[p_tmp_0 + 1] = (xr >> 1).wrapping_sub(accu6);
    tmp[p_tmp_1 + 1] = ((xr >> 1).wrapping_add(accu6)).wrapping_neg();
}

/// `dct_III(pDat, tmp, L, pDat_e)`: the straight port of [`dct_iii`].
pub fn dct_iii_ref(p_dat: &mut [FixpDbl], tmp: &mut [FixpDbl], l: i32, p_dat_e: &mut i32) {
    let m = l >> 1;

    fdk_assert!(l % 4 == 0);
    let (_, sin_twiddle, inc) = dct_get_tables(l);
    let Some(sin_twiddle) = sin_twiddle else { return };
    let inc = inc >> 1;

    let (l, m, inc) = (l as usize, m as usize, inc as usize);
    let tw = |k: usize| sin_twiddle[k];

    let mut p_tmp_0 = 2usize;
    let mut p_tmp_1 = (m - 1) * 2;

    let mut index = 4 * inc;

    // This loop performs multiplication for index i (i*inc)
    let mut i = 1usize;
    while i < m >> 1 {
        let (accu2, accu1) = cplx_mult_div2(p_dat[l - i], p_dat[i], tw(i * inc).re, tw(i * inc).im);
        let w = tw((m - i) * inc);
        let (mut accu4, mut accu3) = cplx_mult_div2(p_dat[m + i], p_dat[m - i], w.re, w.im);
        accu3 >>= 1;
        accu4 >>= 1;

        // This method is better for ARM926, that uses operand2 shifted right by 1 always
        let w = tw(index);
        let (accu6, accu5);
        if 2 * i < (m / 2) {
            (accu6, accu5) = cplx_mult_div2(
                accu3.wrapping_sub(accu1 >> 1),
                (accu2 >> 1).wrapping_add(accu4),
                w.re,
                w.im,
            );
        } else {
            let a6;
            (a6, accu5) = cplx_mult_div2(
                (accu2 >> 1).wrapping_add(accu4),
                accu3.wrapping_sub(accu1 >> 1),
                w.re,
                w.im,
            );
            accu6 = a6.wrapping_neg();
        }
        let mut xr = (accu1 >> 1).wrapping_add(accu3);
        tmp[p_tmp_0] = (xr >> 1).wrapping_sub(accu5);
        tmp[p_tmp_1] = (xr >> 1).wrapping_add(accu5);

        xr = (accu2 >> 1).wrapping_sub(accu4);
        tmp[p_tmp_0 + 1] = (xr >> 1).wrapping_sub(accu6);
        tmp[p_tmp_1 + 1] = ((xr >> 1).wrapping_add(accu6)).wrapping_neg();

        // Create index helper variables for (4*i)*inc indexed equivalent values of short
        // tables.
        if 2 * i < ((m / 2).wrapping_sub(1)) {
            index += 4 * inc;
        } else if 2 * i >= (m / 2) {
            index = index.wrapping_sub(4 * inc);
        }

        i += 1;
        p_tmp_0 += 2;
        p_tmp_1 -= 2;
    }

    let xr = f_mult_div2(p_dat[m], tw(m * inc).re); // cos((PI/(2*L))*M);
    tmp[0] = ((p_dat[0] >> 1).wrapping_add(xr)) >> 1;
    tmp[1] = ((p_dat[0] >> 1).wrapping_sub(xr)) >> 1;

    let w = tw(m * inc / 2);
    let (accu2, accu1) = cplx_mult_div2(p_dat[l - (m / 2)], p_dat[m / 2], w.re, w.im);
    tmp[m] = accu1 >> 1;
    tmp[m + 1] = accu2 >> 1;

    // dit_fft expects 1 bit scaled input values
    crate::fft::fft_ref(m as i32, tmp, p_dat_e);

    // ARM926: 12 cycles per 2-iteration, no overhead code by compiler
    let mut p_tmp_1 = l;
    let mut t = 0usize;
    let mut d = 0usize;
    for _ in 0..(m >> 1) {
        let tmp1 = tmp[t];
        let tmp2 = tmp[t + 1];
        t += 2;
        let tmp3 = tmp[p_tmp_1 - 1];
        let tmp4 = tmp[p_tmp_1 - 2];
        p_tmp_1 -= 2;
        p_dat[d] = tmp1;
        p_dat[d + 1] = tmp3;
        p_dat[d + 2] = tmp2;
        p_dat[d + 3] = tmp4;
        d += 4;
    }

    *p_dat_e += 2;
}

/// `dst_III(pDat, tmp, L, pDat_e)`: DST-III of length `L`, computed by a DCT-III of the mirrored
/// input and sign flipping of the odd output coefficients.
///
/// Note that the factor 0.5 for the sum term `x[N-1]` is 1.0 instead of 0.5. A factor of
/// `sqrt(2/N)` is not applied. `tmp` is a work buffer of at least `L` values.
pub fn dst_iii(p_dat: &mut [FixpDbl], tmp: &mut [FixpDbl], l: i32, p_dat_e: &mut i32) {
    dst_iii_with(p_dat, tmp, l, p_dat_e, dct_iii)
}

/// `dst_III(pDat, tmp, L, pDat_e)` with the straight port [`dct_iii_ref`].
pub fn dst_iii_ref(p_dat: &mut [FixpDbl], tmp: &mut [FixpDbl], l: i32, p_dat_e: &mut i32) {
    dst_iii_with(p_dat, tmp, l, p_dat_e, dct_iii_ref)
}

#[inline(always)]
fn dst_iii_with(
    p_dat: &mut [FixpDbl],
    tmp: &mut [FixpDbl],
    l: i32,
    p_dat_e: &mut i32,
    dct3: fn(&mut [FixpDbl], &mut [FixpDbl], i32, &mut i32),
) {
    let lu = l.max(0) as usize;

    // note: DCT III is reused here, direct DST III implementation might be more efficient

    // mirror input
    p_dat[..lu].reverse();

    // DCT-III
    dct3(p_dat, tmp, l, p_dat_e);

    // flip signs at odd indices
    for i in (1..lu).step_by(2) {
        p_dat[i] = p_dat[i].wrapping_neg();
    }
}

/// `dct_II(pDat, tmp, L, pDat_e)`: DCT-II of length `L` (a multiple of 4). A factor of
/// `sqrt(2/(N-1))` is not applied. `tmp` is a work buffer of at least `L` values.
///
/// The same as the straight port [`dct_ii_ref`], see [`dct_iii`] (`neon::dct_ii_block`).
pub fn dct_ii(p_dat: &mut [FixpDbl], tmp: &mut [FixpDbl], l: i32, p_dat_e: &mut i32) {
    #[cfg(target_arch = "aarch64")]
    if NEON && l == 64 && p_dat.len() >= 64 && tmp.len() >= 64 {
        return dct_ii_neon_64(p_dat, tmp, p_dat_e);
    }
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    if !ARCH_ARM && l == 64 && p_dat.len() >= 64 && tmp.len() >= 64 && crate::simd::avx2() {
        // SAFETY: the CPU has AVX2.
        if unsafe { dct_ii_x86_64(p_dat, tmp, p_dat_e) } {
            return;
        }
    }
    let m = l >> 1;

    fdk_assert!(l % 4 == 0);
    let (_, sin_twiddle, inc) = dct_get_tables(l);
    let Some(sin_twiddle) = sin_twiddle else { return };
    let inc = inc >> 1;

    let (l, m, inc) = (l as usize, m as usize, inc as usize);
    let (p_dat, tmp) = (&mut p_dat[..l], &mut tmp[..l]);
    let tw = |k: usize| sin_twiddle[k];

    #[allow(unused_mut)]
    let mut i = 0usize;
    #[cfg(target_arch = "aarch64")]
    while i + 4 <= m {
        // SAFETY: reads p_dat[2i..2i+8], writes tmp[i..i+4] and tmp[l-4-i..l-i], inside the l
        // values for i + 4 <= m.
        unsafe { neon::dct_ii_pre_block(p_dat.as_ptr(), tmp.as_mut_ptr(), l, i) };
        i += 4;
    }
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    while !ARCH_ARM && i + 4 <= m {
        // SAFETY: as above.
        unsafe { sse2::dct_ii_pre_block(p_dat.as_ptr(), tmp.as_mut_ptr(), l, i) };
        i += 4;
    }
    while i < m {
        tmp[i] = p_dat[2 * i] >> 2;
        tmp[l - 1 - i] = p_dat[2 * i + 1] >> 2;
        i += 1;
    }

    fft(m as i32, tmp, p_dat_e);

    let half = m >> 1;
    #[allow(unused_mut)]
    let mut i = 1usize;
    #[cfg(target_arch = "aarch64")]
    if NEON && half >= 5 {
        // As in dct_III (see there): the iterations read tmp and write p_dat.
        assert!(sin_twiddle.len() > m * inc);
        let split = half.div_ceil(2); // the first i with 2 * i >= m / 2
        while i < half {
            let i0 = i.min(half - 4);
            // SAFETY: the iterations read tmp[2i..2i+2], tmp[2(m-i)..2(m-i)+2] and write
            // p_dat[i], p_dat[l-i], p_dat[m+i], p_dat[m-i] for 1 <= i < m/2, inside the l
            // values; the twiddle indices are checked above.
            unsafe {
                let (x, t, w) = (p_dat.as_mut_ptr(), tmp.as_ptr(), sin_twiddle.as_ptr());
                neon::dct_ii_block(x, t, l, m, i0, w, inc, split);
            }
            i = i0 + 4;
        }
    }
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    if !ARCH_ARM && half >= 5 {
        // As above, with the SSE2 blocks of the other variants (`sse2::dct_ii_block`) and the
        // twiddles of the configuration in a table (`sse2::twiddle_table`).
        assert!(sin_twiddle.len() > m * inc);
        let split = half.div_ceil(2); // the first i with 2 * i >= m / 2
        if let Some(tab) = sse2::twiddle_table(sin_twiddle, m, inc, split) {
            while i < half {
                let i0 = i.min(half - 4);
                // SAFETY: as above; the table holds 3 * (m / 2) words.
                unsafe {
                    let (x, t) = (p_dat.as_mut_ptr(), tmp.as_ptr());
                    sse2::dct_ii_block(x, t, l, m, i0, tab.as_ptr());
                }
                i = i0 + 4;
            }
        }
    }
    while i < half {
        dct_ii_iter(p_dat, tmp, l, m, inc, i, dct_twiddle_index(m, inc, i), sin_twiddle);
        i += 1;
    }

    let w = tw((m / 2) * inc);
    let (accu1, accu2) = cplx_mult(tmp[m], tmp[m + 1], w.re, w.im);
    p_dat[l - (m / 2)] = accu2;
    p_dat[m / 2] = accu1;

    p_dat[0] = tmp[0].wrapping_add(tmp[1]);
    p_dat[m] = f_mult(tmp[0].wrapping_sub(tmp[1]), tw(m * inc).re); // cos((PI/(2*L))*M);

    *p_dat_e += 2;
}

/// The iteration `i` of the loop of [`dct_ii_ref`] after the FFT (`index`: its twiddle index);
/// reads `tmp`, writes `p_dat[i]`, `p_dat[l-i]`, `p_dat[m+i]`, `p_dat[m-i]`.
#[allow(clippy::too_many_arguments)]
#[inline(always)]
fn dct_ii_iter(
    p_dat: &mut [FixpDbl],
    tmp: &[FixpDbl],
    l: usize,
    m: usize,
    inc: usize,
    i: usize,
    index: usize,
    sin_twiddle: &[FixpStp],
) {
    let tw = |k: usize| sin_twiddle[k];
    let (p_tmp_0, p_tmp_1) = (2 * i, 2 * (m - i));
    let mut a1 = (tmp[p_tmp_0 + 1] >> 1).wrapping_add(tmp[p_tmp_1 + 1] >> 1);
    let mut a2 = (tmp[p_tmp_1] >> 1).wrapping_sub(tmp[p_tmp_0] >> 1);

    let w = tw(index);
    let (mut accu1, mut accu2);
    if 2 * i < (m / 2) {
        (accu1, accu2) = cplx_mult_div2(a2, a1, w.re, w.im);
    } else {
        (accu1, accu2) = cplx_mult_div2(a1, a2, w.re, w.im);
        accu1 = accu1.wrapping_neg();
    }
    accu1 <<= 1;
    accu2 <<= 1;

    a1 = (tmp[p_tmp_0] >> 1).wrapping_add(tmp[p_tmp_1] >> 1);
    a2 = (tmp[p_tmp_0 + 1] >> 1).wrapping_sub(tmp[p_tmp_1 + 1] >> 1);

    let w = tw(i * inc);
    let (accu3, accu4) = cplx_mult(accu1.wrapping_add(a2), a1.wrapping_add(accu2), w.re, w.im);
    p_dat[l - i] = accu3.wrapping_neg();
    p_dat[i] = accu4;

    let w = tw((m - i) * inc);
    let (accu3, accu4) = cplx_mult(accu1.wrapping_sub(a2), a1.wrapping_sub(accu2), w.re, w.im);
    p_dat[m + i] = accu3.wrapping_neg();
    p_dat[m - i] = accu4;
}

/// `dct_II(pDat, tmp, L, pDat_e)`: the straight port of [`dct_ii`].
pub fn dct_ii_ref(p_dat: &mut [FixpDbl], tmp: &mut [FixpDbl], l: i32, p_dat_e: &mut i32) {
    let m = l >> 1;

    fdk_assert!(l % 4 == 0);
    let (_, sin_twiddle, inc) = dct_get_tables(l);
    let Some(sin_twiddle) = sin_twiddle else { return };
    let inc = inc >> 1;

    let (l, m, inc) = (l as usize, m as usize, inc as usize);
    let tw = |k: usize| sin_twiddle[k];

    for i in 0..m {
        tmp[i] = p_dat[2 * i] >> 2;
        tmp[l - 1 - i] = p_dat[2 * i + 1] >> 2;
    }

    crate::fft::fft_ref(m as i32, tmp, p_dat_e);

    let mut p_tmp_0 = 2usize;
    let mut p_tmp_1 = (m - 1) * 2;

    let mut index = inc * 4;

    let mut i = 1usize;
    while i < m >> 1 {
        let mut a1 = (tmp[p_tmp_0 + 1] >> 1).wrapping_add(tmp[p_tmp_1 + 1] >> 1);
        let mut a2 = (tmp[p_tmp_1] >> 1).wrapping_sub(tmp[p_tmp_0] >> 1);

        let w = tw(index);
        let (mut accu1, mut accu2);
        if 2 * i < (m / 2) {
            (accu1, accu2) = cplx_mult_div2(a2, a1, w.re, w.im);
        } else {
            (accu1, accu2) = cplx_mult_div2(a1, a2, w.re, w.im);
            accu1 = accu1.wrapping_neg();
        }
        accu1 <<= 1;
        accu2 <<= 1;

        a1 = (tmp[p_tmp_0] >> 1).wrapping_add(tmp[p_tmp_1] >> 1);
        a2 = (tmp[p_tmp_0 + 1] >> 1).wrapping_sub(tmp[p_tmp_1 + 1] >> 1);

        let w = tw(i * inc);
        let (accu3, accu4) = cplx_mult(accu1.wrapping_add(a2), a1.wrapping_add(accu2), w.re, w.im);
        p_dat[l - i] = accu3.wrapping_neg();
        p_dat[i] = accu4;

        let w = tw((m - i) * inc);
        let (accu3, accu4) = cplx_mult(accu1.wrapping_sub(a2), a1.wrapping_sub(accu2), w.re, w.im);
        p_dat[m + i] = accu3.wrapping_neg();
        p_dat[m - i] = accu4;

        // Create index helper variables for (4*i)*inc indexed equivalent values of short
        // tables.
        if 2 * i < ((m / 2).wrapping_sub(1)) {
            index += 4 * inc;
        } else if 2 * i >= (m / 2) {
            index = index.wrapping_sub(4 * inc);
        }

        i += 1;
        p_tmp_0 += 2;
        p_tmp_1 -= 2;
    }

    let w = tw((m / 2) * inc);
    let (accu1, accu2) = cplx_mult(tmp[m], tmp[m + 1], w.re, w.im);
    p_dat[l - (m / 2)] = accu2;
    p_dat[m / 2] = accu1;

    p_dat[0] = tmp[0].wrapping_add(tmp[1]);
    p_dat[m] = f_mult(tmp[0].wrapping_sub(tmp[1]), tw(m * inc).re); // cos((PI/(2*L))*M);

    *p_dat_e += 2;
}

/// `WTC(0x5a82799a)`
const W_PI_FOURTH: FixpSgl = wtc(0x5a82799a);

/// `dct_IV(pDat, L, pDat_e)`: DCT-IV of length `L`. A factor of `sqrt(2/N)` is not applied.
///
/// The same values as the straight port [`dct_iv_ref`], with the twiddling loops written per pair
/// of complex values (see `dct_iv_pre_iter`, `dct_iv_post_pair`); on `aarch64` targets four
/// iterations run at a time in NEON (`neon`).
pub fn dct_iv(p_dat: &mut [FixpDbl], l: i32, p_dat_e: &mut i32) {
    dct_iv_dst_iv::<false>(p_dat, l, p_dat_e)
}

/// `dst_IV(pDat, L, pDat_e)`: DST-IV of length `L`. A factor of `sqrt(2/N)` is not applied.
///
/// The same values as the straight port [`dst_iv_ref`], see [`dct_iv`].
pub fn dst_iv(p_dat: &mut [FixpDbl], l: i32, p_dat_e: &mut i32) {
    dct_iv_dst_iv::<true>(p_dat, l, p_dat_e)
}

#[cfg(target_arch = "aarch64")]
pub(crate) mod neon;
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
mod sse2;
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
mod x86;

/// Whether the NEON blocks are used: on `aarch64` targets for the `aarch64` variant (the pre
/// twiddling uses its `cplxMultDiv2`).
const NEON: bool = cfg!(target_arch = "aarch64") && ARCH_ARM;

/// The pre twiddling iteration `i` (even) of `dct_IV` (`DST` false) or `dst_IV` (`DST` true):
/// the body of the first loop of [`dct_iv_ref`] / [`dst_iv_ref`]. It reads and writes
/// `x[i]`, `x[i+1]`, `x[l-2-i]` and `x[l-1-i]` only.
#[inline(always)]
fn dct_iv_pre_iter<const DST: bool>(x: &mut [FixpDbl], l: usize, i: usize, twiddle: &[FixpWtp]) {
    let (p0, p1) = (i, l - 2 - i);
    if !DST {
        let (accu1, accu2) = cplx_mult_div2(x[p1 + 1], x[p0], twiddle[i].re, twiddle[i].im);
        let (accu3, accu4) = cplx_mult_div2(x[p1], x[p0 + 1], twiddle[i + 1].re, twiddle[i + 1].im);
        x[p0] = accu2 >> 1;
        x[p0 + 1] = accu1 >> 1;
        x[p1] = accu4 >> 1;
        x[p1 + 1] = (accu3 >> 1).wrapping_neg();
    } else {
        let accu1 = x[p1 + 1] >> 1;
        let accu2 = (x[p0] >> 1).wrapping_neg();
        let accu3 = x[p0 + 1] >> 1;
        let accu4 = (x[p1] >> 1).wrapping_neg();
        let (accu1, accu2) = cplx_mult_div2(accu1, accu2, twiddle[i].re, twiddle[i].im);
        let (accu3, accu4) = cplx_mult_div2(accu4, accu3, twiddle[i + 1].re, twiddle[i + 1].im);
        x[p0] = accu2;
        x[p0 + 1] = accu1;
        x[p1] = accu4;
        x[p1 + 1] = accu3.wrapping_neg();
    }
}

/// The pair `j < k_end` of the post twiddling of `dct_IV` (`DST` false) or `dst_IV` (`DST`
/// true) of length `l = 2 * mu`, with `k_end = (mu + 1) / 2` (the loop of the straight port runs
/// `k = 1 .. k_end`).
///
/// The loop of the straight port reads every value before it overwrites it, so it computes its
/// output from the FFT output `X` alone. Per pair `j` of the front values `F = (X[2j], X[2j+1])`
/// and the back values `B = (X[l-2-2j], X[l-1-2j])` (the same pair for `j = k_end - 1` if `mu` is
/// odd), its iterations write:
///
/// - `x[2j]` and `x[l-1-2j]` from `cplxMult(F.im, F.re, t[j])` (iteration `k = j`, `j >= 1`);
///   for `j = 0`, `x[0]` and `x[l-1]` are copies of `F` (`dct_IV`: `x[l-1] = -X[1]`;
///   `dst_IV`: `x[0] = X[1]`, `x[l-1] = -X[0]`);
/// - `x[2j+1]` and `x[l-2-2j]` from `cplxMult(B.re, B.im, t[j + 1])` (iteration `k = j + 1`,
///   `j <= k_end - 2`), or from `fMult(B, W_PiFOURTH)` after the loop (`j = k_end - 1`, `mu`
///   even).
///
/// `t[k]` is `sin_twiddle[k * sin_step]`. The pair reads `F` and `B` before it writes.
#[inline(always)]
fn dct_iv_post_pair<const DST: bool>(
    x: &mut [FixpDbl],
    l: usize,
    mu: usize,
    j: usize,
    sin_twiddle: &[FixpStp],
    sin_step: usize,
) {
    let k_end = (mu + 1) >> 1;
    let (f0, f1) = (2 * j, 2 * j + 1);
    let (b0, b1) = (l - 2 - 2 * j, l - 1 - 2 * j);
    let (fe, fo, be, bo) = (x[f0], x[f1], x[b0], x[b1]);

    if j == 0 {
        if !DST {
            x[l - 1] = fo.wrapping_neg();
        } else {
            x[0] = fo;
            x[l - 1] = fe.wrapping_neg();
        }
    } else {
        let twd = sin_twiddle[j * sin_step];
        let (accu3, accu4) = cplx_mult(fo, fe, twd.re, twd.im);
        if !DST {
            x[f0] = accu4;
            x[b1] = accu3.wrapping_neg();
        } else {
            x[f0] = accu3;
            x[b1] = accu4.wrapping_neg();
        }
    }

    if j + 2 <= k_end {
        let twd = sin_twiddle[(j + 1) * sin_step];
        let (accu3, accu4) = cplx_mult(be, bo, twd.re, twd.im);
        if !DST {
            x[f1] = accu3;
            x[b0] = accu4;
        } else {
            x[b0] = accu3.wrapping_neg();
            x[f1] = accu4.wrapping_neg();
        }
    } else if (mu & 1) == 0 {
        // Last Sin and Cos value pair are the same
        let accu1 = f_mult(be, W_PI_FOURTH);
        let accu2 = f_mult(bo, W_PI_FOURTH);
        if !DST {
            x[b0] = accu1.wrapping_add(accu2);
            x[f1] = accu1.wrapping_sub(accu2);
        } else {
            x[f1] = accu1.wrapping_neg().wrapping_sub(accu2);
            x[b0] = accu2.wrapping_sub(accu1);
        }
    }
}

/// The pre twiddling iterations `it0 .. n_iter` (`i = 2 it`, see [`dct_iv_pre_iter`]) on slices
/// of the front values `x[i], x[i+1]` and the back values `x[l-2-i], x[l-1-i]` (no index
/// checks per iteration).
#[inline(always)]
fn dct_iv_pre_range<const DST: bool>(
    x: &mut [FixpDbl],
    l: usize,
    it0: usize,
    n_iter: usize,
    twiddle: &[FixpWtp],
) {
    if it0 >= n_iter {
        return;
    }
    let mu = l / 2;
    let (front, back) = x[..l].split_at_mut(mu);
    let f = front[2 * it0..2 * n_iter].chunks_exact_mut(2);
    let b = back[mu - 2 * n_iter..mu - 2 * it0].rchunks_exact_mut(2);
    let t = twiddle[2 * it0..2 * n_iter].chunks_exact(2);
    for ((f, b), t) in f.zip(b).zip(t) {
        if !DST {
            let (accu1, accu2) = cplx_mult_div2(b[1], f[0], t[0].re, t[0].im);
            let (accu3, accu4) = cplx_mult_div2(b[0], f[1], t[1].re, t[1].im);
            f[0] = accu2 >> 1;
            f[1] = accu1 >> 1;
            b[0] = accu4 >> 1;
            b[1] = (accu3 >> 1).wrapping_neg();
        } else {
            let accu1 = b[1] >> 1;
            let accu2 = (f[0] >> 1).wrapping_neg();
            let accu3 = f[1] >> 1;
            let accu4 = (b[0] >> 1).wrapping_neg();
            let (accu1, accu2) = cplx_mult_div2(accu1, accu2, t[0].re, t[0].im);
            let (accu3, accu4) = cplx_mult_div2(accu4, accu3, t[1].re, t[1].im);
            f[0] = accu2;
            f[1] = accu1;
            b[0] = accu4;
            b[1] = accu3.wrapping_neg();
        }
    }
}

/// The post twiddling pairs `j0 .. j1` (`1 <= j0`, `j1 <= k_end - 1`, so both formulas of
/// [`dct_iv_post_pair`] apply) on slices of the front values `x[2j], x[2j+1]` and the back
/// values `x[l-2-2j], x[l-1-2j]` (no index checks per pair).
#[inline(always)]
fn dct_iv_post_range<const DST: bool>(
    x: &mut [FixpDbl],
    l: usize,
    j0: usize,
    j1: usize,
    sin_twiddle: &[FixpStp],
    sin_step: usize,
) {
    if j0 >= j1 {
        return;
    }
    let mu = l / 2;
    let (front, back) = x[..l].split_at_mut(mu);
    let f = front[2 * j0..2 * j1].chunks_exact_mut(2);
    let b = back[mu - 2 * j1..mu - 2 * j0].rchunks_exact_mut(2);
    // t[j * sin_step] for j = j0 ..= j1
    let mut tws = sin_twiddle[j0 * sin_step..=j1 * sin_step].iter().step_by(sin_step);
    let mut cur = *tws.next().unwrap_or(&FixpStp::default());
    for ((f, b), nxt) in f.zip(b).zip(tws) {
        let (fe, fo, be, bo) = (f[0], f[1], b[0], b[1]);
        let (accu3, accu4) = cplx_mult(fo, fe, cur.re, cur.im);
        let (bccu3, bccu4) = cplx_mult(be, bo, nxt.re, nxt.im);
        if !DST {
            f[0] = accu4;
            b[1] = accu3.wrapping_neg();
            f[1] = bccu3;
            b[0] = bccu4;
        } else {
            f[0] = accu3;
            b[1] = accu4.wrapping_neg();
            b[0] = bccu3.wrapping_neg();
            f[1] = bccu4.wrapping_neg();
        }
        cur = *nxt;
    }
}

/// [`dct_iv`] (`DST` false) and [`dst_iv`] (`DST` true).
#[inline(always)]
fn dct_iv_dst_iv<const DST: bool>(p_dat: &mut [FixpDbl], l: i32, p_dat_e: &mut i32) {
    #[cfg(target_arch = "aarch64")]
    if NEON && (l == 32 || l == 64) && p_dat.len() >= l as usize {
        if l == 32 {
            return dct_iv_dst_iv_neon_n::<DST, 32>(p_dat, p_dat_e);
        }
        return dct_iv_dst_iv_neon_n::<DST, 64>(p_dat, p_dat_e);
    }
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    if !ARCH_ARM && (l == 32 || l == 64) && p_dat.len() >= l as usize && crate::simd::avx2() {
        // SAFETY: the CPU has AVX2.
        unsafe {
            if l == 32 {
                return dct_iv_dst_iv_x86_n::<DST, 32>(p_dat, p_dat_e);
            }
            return dct_iv_dst_iv_x86_n::<DST, 64>(p_dat, p_dat_e);
        }
    }
    let m = l >> 1;

    fdk_assert!(l >= 4);

    let (twiddle, sin_twiddle, sin_step) = dct_get_tables(l);
    let (Some(twiddle), Some(sin_twiddle)) = (twiddle, sin_twiddle) else { return };

    let (l, mu, sin_step) = (l as usize, m as usize, sin_step as usize);
    let x = &mut p_dat[..l];

    // Pre twiddling: the iterations i = 0, 2, .. < mu - 1 touch disjoint values.
    let n_iter = mu / 2;
    #[allow(unused_mut)]
    let mut it = 0usize;
    if NEON {
        #[cfg(target_arch = "aarch64")]
        {
            // The iterations it .. it + 4 read twiddle[2 it .. 2 it + 8] and x[2 it .. 2 it + 8],
            // x[l - 8 - 2 it .. l - 2 it], within the arrays for it + 4 <= n_iter <= mu / 2.
            assert!(twiddle.len() >= 2 * n_iter);
            while it + 4 <= n_iter {
                // SAFETY: see above; the front and back values do not overlap (2 it + 8 <= mu).
                unsafe {
                    if DST {
                        neon::dst_iv_pre_block(x.as_mut_ptr(), l, 2 * it, twiddle.as_ptr());
                    } else {
                        neon::dct_iv_pre_block(x.as_mut_ptr(), l, 2 * it, twiddle.as_ptr());
                    }
                }
                it += 4;
            }
        }
    }
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    if !ARCH_ARM && crate::fft_rad2::x86::avx2() {
        // Eight iterations at a time (see `x86::pre_block`): they read twiddle[2 it .. 2 it + 16]
        // and the complex values it .. it + 8, mu - 8 - it .. mu - it of x, within the arrays
        // for it + 8 <= n_iter <= mu / 2.
        assert!(twiddle.len() >= 2 * n_iter);
        while it + 8 <= n_iter {
            // SAFETY: see above; AVX2 is available.
            unsafe { x86::pre_block::<DST>(x.as_mut_ptr(), mu, it, twiddle.as_ptr()) };
            it += 8;
        }
    }
    dct_iv_pre_range::<DST>(x, l, it, n_iter, twiddle);
    if mu & 1 != 0 {
        let i = 2 * n_iter;
        let (p0, p1) = (i, l - 2 - i);
        let tw = twiddle[i];
        if !DST {
            let (accu1, accu2) = cplx_mult_div2(x[p1 + 1], x[p0], tw.re, tw.im);
            x[p0] = accu2 >> 1;
            x[p0 + 1] = accu1 >> 1;
        } else {
            let (accu1, accu2) = cplx_mult_div2(x[p1 + 1], x[p0].wrapping_neg(), tw.re, tw.im);
            x[p0] = accu2 >> 1;
            x[p0 + 1] = accu1 >> 1;
        }
    }

    fft(m, x, p_dat_e);

    // Post twiddling, per pair (see `dct_iv_post_pair`).
    let k_end = (mu + 1) >> 1;
    #[cfg(target_arch = "aarch64")]
    if NEON && mu % 2 == 0 && k_end >= 8 {
        // All pairs in NEON blocks: the first block includes the pair 0, the last block the
        // pair k_end - 1 (see `neon::post_block`), the blocks between start at 4, 8, ...; the
        // pairs between the regular blocks and the last block (k_end not a multiple of 4) are
        // scalar. The blocks read sin_twiddle[j * sin_step] for j < k_end and x[2 j .. 2 j + 8],
        // x[l - 8 - 2 j .. l - 2 j] for j + 3 < k_end, within the arrays (the front and back
        // values do not overlap: 2 k_end = mu).
        assert!(sin_twiddle.len() > (k_end - 1) * sin_step);
        let (xp, tp) = (x.as_mut_ptr(), sin_twiddle.as_ptr());
        // SAFETY: see above.
        unsafe { neon::post_block::<DST, true, false>(xp, l, 0, tp, sin_step) };
        let mut j = 4usize;
        while j + 4 <= k_end - 4 {
            // SAFETY: see above.
            unsafe { neon::post_block::<DST, false, false>(xp, l, j, tp, sin_step) };
            j += 4;
        }
        dct_iv_post_range::<DST>(x, l, j, k_end - 4, sin_twiddle, sin_step);
        let xp = x.as_mut_ptr();
        // SAFETY: see above.
        unsafe { neon::post_block::<DST, false, true>(xp, l, k_end - 4, tp, sin_step) };
        // Add twiddeling scale.
        *p_dat_e += 2;
        return;
    }
    dct_iv_post_pair::<DST>(x, l, mu, 0, sin_twiddle, sin_step);
    #[allow(unused_mut)]
    let mut j = 1usize;
    if NEON {
        #[cfg(target_arch = "aarch64")]
        {
            // The pairs j .. j + 4 read sin_twiddle[j * sin_step .. (j + 4) * sin_step] and
            // x[2 j .. 2 j + 8], x[l - 8 - 2 j .. l - 2 j], within the arrays for
            // j + 4 <= k_end - 1 (the straight loop reads sin_twiddle[(k_end - 1) * sin_step]).
            if k_end >= 1 {
                assert!(sin_twiddle.len() > (k_end - 1) * sin_step);
            }
            while j + 5 <= k_end {
                // SAFETY: see above; the front and back values do not overlap (2 j + 8 <= mu).
                unsafe {
                    neon::post_block::<DST, false, false>(
                        x.as_mut_ptr(),
                        l,
                        j,
                        sin_twiddle.as_ptr(),
                        sin_step,
                    );
                }
                j += 4;
            }
        }
    }
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    if !ARCH_ARM && crate::fft_rad2::x86::avx2() {
        // Eight pairs at a time (see `x86::post_block`): they read sin_twiddle[j * sin_step ..=
        // (j + 8) * sin_step] and the complex values j .. j + 8, mu - 8 - j .. mu - j of x,
        // within the arrays for j + 9 <= k_end (the straight loop reads
        // sin_twiddle[(k_end - 1) * sin_step]).
        if k_end >= 1 {
            assert!(sin_twiddle.len() > (k_end - 1) * sin_step);
        }
        while j + 9 <= k_end {
            // SAFETY: see above; AVX2 is available.
            unsafe {
                x86::post_block::<DST>(x.as_mut_ptr(), mu, j, sin_twiddle.as_ptr(), sin_step)
            };
            j += 8;
        }
    }
    if k_end >= 2 {
        dct_iv_post_range::<DST>(x, l, j.min(k_end - 1), k_end - 1, sin_twiddle, sin_step);
        dct_iv_post_pair::<DST>(x, l, mu, k_end - 1, sin_twiddle, sin_step);
    }

    // Add twiddeling scale.
    *p_dat_e += 2;
}

/// [`dct_iv_dst_iv`] for the lengths `L = 32` and `64` (the QMF filter banks) on `aarch64`
/// targets (`aarch64` variant): the NEON blocks and the FFT that the general code runs for these
/// lengths (all pre twiddling iterations in blocks of four, the FFT of length `L / 2`, all post
/// twiddling pairs in blocks of four with the special pairs in the first and the last block),
/// with the values kept in registers between the three steps. A pre twiddling block produces the
/// complex values `4b .. 4b + 4` and `L/2 - 4 - 4b .. L/2 - 4b` (the vectors `b` and
/// `L/8 - 1 - b` of the FFT input), the FFT takes and returns vectors of 4 consecutive complex
/// values, and the post twiddling block `j0 = 4b` reads the vectors `b` and `L/8 - 1 - b` of its
/// output: the same values that the general code stores and loads in between.
#[cfg(target_arch = "aarch64")]
#[inline(never)]
fn dct_iv_dst_iv_neon_n<const DST: bool, const L: usize>(p_dat: &mut [FixpDbl], p_dat_e: &mut i32) {
    use crate::fft::neon16::{fft_16_regs, fft_32_regs};
    use core::arch::aarch64::vdupq_n_s32;
    const { assert!(L == 32 || L == 64) };
    let (twiddle, sin_twiddle, sin_step) = dct_get_tables(L as i32);
    let (Some(twiddle), Some(sin_twiddle)) = (twiddle, sin_twiddle) else { return };
    let sin_step = sin_step as usize;
    let mu = L / 2;
    let k_end = mu / 2; // (mu + 1) >> 1, mu even
    let nv = L / 8; // complex vectors of 4 values
    let x: &mut [FixpDbl; L] = (&mut p_dat[..L]).try_into().unwrap();
    // the blocks read twiddle[0 .. mu] and sin_twiddle[j * sin_step] for j < k_end
    assert!(twiddle.len() >= mu && sin_twiddle.len() > (k_end - 1) * sin_step);
    let (tw, st) = (twiddle.as_ptr(), sin_twiddle.as_ptr());
    let xp = x.as_mut_ptr();
    // SAFETY: NEON is part of AArch64; as in `dct_iv_dst_iv` for these lengths, the pre blocks
    // at 2 it (it = 4b < mu / 2) and the post blocks at j = 4b < k_end stay inside x and the
    // tables.
    unsafe {
        let z = vdupq_n_s32(0);
        let mut v = [neon::C(z, z); 8];
        for b in 0..nv / 2 {
            let (f, bk) = if DST {
                neon::dst_iv_pre_regs(xp, L, 8 * b, tw)
            } else {
                neon::dct_iv_pre_regs(xp, L, 8 * b, tw)
            };
            v[b] = f;
            v[nv - 1 - b] = bk;
        }
        if L == 32 {
            let r = fft_16_regs([v[0], v[1], v[2], v[3]]);
            v[..4].copy_from_slice(&r);
            *p_dat_e += crate::fft::SCALEFACTOR16;
        } else {
            v = fft_32_regs(v);
            *p_dat_e += crate::fft::SCALEFACTOR32;
        }
        neon::post_block_regs::<DST, true, false>(v[0], v[nv - 1], xp, L, 0, st, sin_step);
        for b in 1..nv / 2 - 1 {
            neon::post_block_regs::<DST, false, false>(
                v[b],
                v[nv - 1 - b],
                xp,
                L,
                4 * b,
                st,
                sin_step,
            );
        }
        let b = nv / 2 - 1;
        neon::post_block_regs::<DST, false, true>(v[b], v[nv - 1 - b], xp, L, 4 * b, st, sin_step);
    }
    // Add twiddeling scale.
    *p_dat_e += 2;
}

/// [`dct_iii`] of 32 values on `aarch64` targets (`aarch64` variant, the QMF analysis of the
/// low power SBR): the operations of the general code for this length (the NEON blocks of the
/// iterations `1..5` and `4..8`, the two single values, `fft_16`, the output permutation in two
/// blocks) with the FFT input and output in registers. The FFT input vectors (complex values
/// `4q .. 4q + 4` of `tmp`) are: `(t0, front 1..4)`, front `4..8`, `(t8, back 9..12)`, back
/// `12..16`, where `t0`, `t8` are the single values and front/back the values the blocks write
/// (the value 4 and 12 is written by both blocks, with the same result). The FFT output is also
/// stored to `tmp`, which then holds what the general code leaves there.
#[cfg(target_arch = "aarch64")]
#[inline(never)]
fn dct_iii_neon_32(p_dat: &mut [FixpDbl], tmp: &mut [FixpDbl], p_dat_e: &mut i32) {
    use crate::fft::neon16::fft_16_regs;
    use core::arch::aarch64::*;
    const L: usize = 32;
    const M: usize = 16;
    let (_, sin_twiddle, inc) = dct_get_tables(L as i32);
    let Some(sin_twiddle) = sin_twiddle else { return };
    let inc = (inc >> 1) as usize;
    let split = (M / 2).div_ceil(2); // the first i with 2 * i >= m / 2
    assert!(sin_twiddle.len() > M * inc);
    let (p_dat, tmp) = (&mut p_dat[..L], &mut tmp[..L]);
    let tw = |k: usize| sin_twiddle[k];

    // the single values: complex 0 and m / 2 of tmp
    let xr = f_mult_div2(p_dat[M], tw(M * inc).re); // cos((PI/(2*L))*M);
    let t0 = ((p_dat[0] >> 1).wrapping_add(xr)) >> 1;
    let t1 = ((p_dat[0] >> 1).wrapping_sub(xr)) >> 1;
    let w = tw(M * inc / 2);
    let (accu2, accu1) = cplx_mult_div2(p_dat[L - (M / 2)], p_dat[M / 2], w.re, w.im);
    let (t16, t17) = (accu1 >> 1, accu2 >> 1);

    // SAFETY: NEON is part of AArch64; the blocks read p_dat[i], p_dat[l-i], p_dat[m+i],
    // p_dat[m-i] for 1 <= i < 8 and the twiddles up to m * inc (checked above), the stores stay
    // inside the 32 values of tmp and p_dat.
    unsafe {
        let (x, t) = (p_dat.as_ptr(), sin_twiddle.as_ptr());
        let (fa, ba) = neon::dct_iii_block_regs(x, L, M, 1, t, inc, split);
        let (fb, bb) = neon::dct_iii_block_regs(x, L, M, 4, t, inc, split);
        let ins = |a: i32, v: int32x4_t| vextq_s32::<3>(vdupq_n_s32(a), v);
        let v = [
            neon::C(ins(t0, fa.0), ins(t1, fa.1)),
            fb,
            neon::C(ins(t16, bb.0), ins(t17, bb.1)),
            ba,
        ];
        let v = fft_16_regs(v);
        let tp = tmp.as_mut_ptr();
        for (q, c) in v.iter().enumerate() {
            vst2q_s32(tp.add(8 * q), int32x4x2_t(c.0, c.1));
        }
        let xp = p_dat.as_mut_ptr();
        neon::dct_iii_post_regs(xp, 0, v[0], v[3]);
        neon::dct_iii_post_regs(xp, 4, v[1], v[2]);
    }
    *p_dat_e += crate::fft::SCALEFACTOR16;
    *p_dat_e += 2;
}

/// [`dct_ii`] of 64 values on `aarch64` targets (`aarch64` variant, the QMF synthesis of the
/// low power SBR): the operations of the general code for this length with the FFT input and
/// output in registers. The input permutation `tmp[i] = x[2i] >> 2`, `tmp[l-1-i] = x[2i+1] >> 2`
/// gives the FFT input vectors directly from 4-way deinterleaved loads of `x`: the complex values
/// `4q .. 4q + 4` of `tmp` are `(e0 >> 2, e2 >> 2)` of `x[16q .. 16q + 16]` (`e_k`: the values
/// `16q + 4r + k`), the complex values `28 - 4q .. 32 - 4q` are `(e3 >> 2, e1 >> 2)` of the same
/// load, reversed. The blocks after the FFT (`i0 = 1, 5, 9, 12` as in the general code) take the
/// complex values `i0 .. i0 + 4` and `m - i0 - 3 .. m - i0 + 1` from the output vectors (shifted
/// by one value where `i0` is not a multiple of 4), the single values from their lanes. The FFT
/// output is also stored to `tmp`, which then holds what the general code leaves there.
#[cfg(target_arch = "aarch64")]
#[inline(never)]
fn dct_ii_neon_64(p_dat: &mut [FixpDbl], tmp: &mut [FixpDbl], p_dat_e: &mut i32) {
    use crate::fft::neon16::fft_32_regs;
    use core::arch::aarch64::*;
    const L: usize = 64;
    const M: usize = 32;
    let (_, sin_twiddle, inc) = dct_get_tables(L as i32);
    let Some(sin_twiddle) = sin_twiddle else { return };
    let inc = (inc >> 1) as usize;
    let split = (M / 2).div_ceil(2); // the first i with 2 * i >= m / 2
    assert!(sin_twiddle.len() > M * inc);
    let (p_dat, tmp) = (&mut p_dat[..L], &mut tmp[..L]);
    let tw = |k: usize| sin_twiddle[k];
    // SAFETY: NEON is part of AArch64; the loads read the 64 values of p_dat, the stores write
    // the 64 values of tmp and p_dat[i], p_dat[l-i], p_dat[m+i], p_dat[m-i] for 1 <= i < 16; the
    // twiddle indices are at most m * inc (checked above).
    let (v0, v4) = unsafe {
        let z = vdupq_n_s32(0);
        let mut v = [neon::C(z, z); 8];
        let xp = p_dat.as_ptr();
        for q in 0..4 {
            let e = vld4q_s32(xp.add(16 * q));
            v[q] = neon::C(vshrq_n_s32::<2>(e.0), vshrq_n_s32::<2>(e.2));
            v[7 - q] = neon::C(neon::rev(vshrq_n_s32::<2>(e.3)), neon::rev(vshrq_n_s32::<2>(e.1)));
        }
        let v = fft_32_regs(v);
        let tp = tmp.as_mut_ptr();
        for (q, c) in v.iter().enumerate() {
            vst2q_s32(tp.add(8 * q), int32x4x2_t(c.0, c.1));
        }
        let ext1 =
            |a: neon::C, b: neon::C| neon::C(vextq_s32::<1>(a.0, b.0), vextq_s32::<1>(a.1, b.1));
        let (x, t) = (p_dat.as_mut_ptr(), sin_twiddle.as_ptr());
        neon::dct_ii_block_regs(ext1(v[0], v[1]), v[7], x, L, M, 1, t, inc, split);
        neon::dct_ii_block_regs(ext1(v[1], v[2]), v[6], x, L, M, 5, t, inc, split);
        neon::dct_ii_block_regs(ext1(v[2], v[3]), v[5], x, L, M, 9, t, inc, split);
        neon::dct_ii_block_regs(v[3], ext1(v[4], v[5]), x, L, M, 12, t, inc, split);
        (v[0], v[4])
    };
    *p_dat_e += crate::fft::SCALEFACTOR32;
    // the single values (complex 0 and m / 2 of tmp)
    // SAFETY: NEON lane reads.
    let (tm0, tm1, t0, t1) = unsafe {
        (
            vgetq_lane_s32::<0>(v4.0),
            vgetq_lane_s32::<0>(v4.1),
            vgetq_lane_s32::<0>(v0.0),
            vgetq_lane_s32::<0>(v0.1),
        )
    };
    let w = tw((M / 2) * inc);
    let (accu1, accu2) = cplx_mult(tm0, tm1, w.re, w.im);
    p_dat[L - (M / 2)] = accu2;
    p_dat[M / 2] = accu1;

    p_dat[0] = t0.wrapping_add(t1);
    p_dat[M] = f_mult(t0.wrapping_sub(t1), tw(M * inc).re); // cos((PI/(2*L))*M);

    *p_dat_e += 2;
}

/// The x86 form of [`dct_iii_neon_32`] for the variants other than `aarch64` (selected at run
/// time with AVX2): the SSE2 blocks of `sse2` and the vector FFT of `crate::fft::x86_16` with the
/// FFT input and output in registers. Returns `false` (nothing done) if the twiddle table cache
/// of `sse2` is full; the general code then runs.
///
/// # Safety
///
/// AVX2 must be available.
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[target_feature(enable = "avx2")]
unsafe fn dct_iii_x86_32(p_dat: &mut [FixpDbl], tmp: &mut [FixpDbl], p_dat_e: &mut i32) -> bool {
    use crate::fft::x86_16::{fft_16_regs, st_at};
    #[cfg(target_arch = "x86")]
    use core::arch::x86::*;
    #[cfg(target_arch = "x86_64")]
    use core::arch::x86_64::*;
    use sse2::C;
    const L: usize = 32;
    const M: usize = 16;
    let (_, sin_twiddle, inc) = dct_get_tables(L as i32);
    let Some(sin_twiddle) = sin_twiddle else { return false };
    let inc = (inc >> 1) as usize;
    let split = (M / 2).div_ceil(2); // the first i with 2 * i >= m / 2
    assert!(sin_twiddle.len() > M * inc);
    let Some(tab) = sse2::twiddle_table(sin_twiddle, M, inc, split) else { return false };
    let (p_dat, tmp) = (&mut p_dat[..L], &mut tmp[..L]);
    let tw = |k: usize| sin_twiddle[k];

    // the single values: complex 0 and m / 2 of tmp
    let xr = f_mult_div2(p_dat[M], tw(M * inc).re); // cos((PI/(2*L))*M);
    let t0 = ((p_dat[0] >> 1).wrapping_add(xr)) >> 1;
    let t1 = ((p_dat[0] >> 1).wrapping_sub(xr)) >> 1;
    let w = tw(M * inc / 2);
    let (accu2, accu1) = cplx_mult_div2(p_dat[L - (M / 2)], p_dat[M / 2], w.re, w.im);
    let (t16, t17) = (accu1 >> 1, accu2 >> 1);

    // SAFETY: AVX2 (and so SSSE3) is available; as in `dct_iii_neon_32`, the blocks stay inside
    // the 32 values of p_dat and tmp; the table holds 3 * (m / 2) words.
    unsafe {
        let x = p_dat.as_ptr();
        let (fa, ba) = sse2::dct_iii_block_regs(x, L, M, 1, tab.as_ptr());
        let (fb, bb) = sse2::dct_iii_block_regs(x, L, M, 4, tab.as_ptr());
        // (a, v[0], v[1], v[2])
        let ins = |a: i32, v: __m128i| _mm_alignr_epi8::<12>(v, _mm_set1_epi32(a));
        let v = [C(ins(t0, fa.0), ins(t1, fa.1)), fb, C(ins(t16, bb.0), ins(t17, bb.1)), ba];
        let v = fft_16_regs(v);
        let tp = tmp.as_mut_ptr();
        for (q, c) in v.iter().enumerate() {
            st_at(tp, 4 * q, *c);
        }
        let xp = p_dat.as_mut_ptr();
        sse2::dct_iii_post_regs(xp, 0, v[0], v[3]);
        sse2::dct_iii_post_regs(xp, 4, v[1], v[2]);
    }
    *p_dat_e += crate::fft::SCALEFACTOR16;
    *p_dat_e += 2;
    true
}

/// The x86 form of [`dct_ii_neon_64`] (see [`dct_iii_x86_32`]); the 4-way deinterleaved loads
/// are 4x4 transposes. Returns `false` (nothing done) if the twiddle table cache is full.
///
/// # Safety
///
/// AVX2 must be available.
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[target_feature(enable = "avx2")]
unsafe fn dct_ii_x86_64(p_dat: &mut [FixpDbl], tmp: &mut [FixpDbl], p_dat_e: &mut i32) -> bool {
    use crate::fft::x86_16::{fft_32_regs, st_at};
    #[cfg(target_arch = "x86")]
    use core::arch::x86::*;
    #[cfg(target_arch = "x86_64")]
    use core::arch::x86_64::*;
    use sse2::C;
    const L: usize = 64;
    const M: usize = 32;
    let (_, sin_twiddle, inc) = dct_get_tables(L as i32);
    let Some(sin_twiddle) = sin_twiddle else { return false };
    let inc = (inc >> 1) as usize;
    let split = (M / 2).div_ceil(2); // the first i with 2 * i >= m / 2
    assert!(sin_twiddle.len() > M * inc);
    let Some(tab) = sse2::twiddle_table(sin_twiddle, M, inc, split) else { return false };
    let (p_dat, tmp) = (&mut p_dat[..L], &mut tmp[..L]);
    let tw = |k: usize| sin_twiddle[k];
    // SAFETY: AVX2 (and so SSSE3) is available; the loads read the 64 values of p_dat, the
    // stores write the 64 values of tmp and p_dat[i], p_dat[l-i], p_dat[m+i], p_dat[m-i] for
    // 1 <= i < 16; the table holds 3 * (m / 2) words.
    let (t0, t1, tm0, tm1) = unsafe {
        let rev = |v: __m128i| _mm_shuffle_epi32::<0b00_01_10_11>(v);
        let z = _mm_setzero_si128();
        let mut v = [C(z, z); 8];
        let xp = p_dat.as_ptr();
        for q in 0..4 {
            let r = |k: usize| _mm_loadu_si128(xp.add(16 * q + 4 * k) as *const __m128i);
            let (r0, r1, r2, r3) = (r(0), r(1), r(2), r(3));
            // the columns e_k (values 16q + 4r + k) of the 4x4 block
            let a0 = _mm_unpacklo_epi32(r0, r1);
            let a1 = _mm_unpacklo_epi32(r2, r3);
            let a2 = _mm_unpackhi_epi32(r0, r1);
            let a3 = _mm_unpackhi_epi32(r2, r3);
            let e0 = _mm_unpacklo_epi64(a0, a1);
            let e1 = _mm_unpackhi_epi64(a0, a1);
            let e2 = _mm_unpacklo_epi64(a2, a3);
            let e3 = _mm_unpackhi_epi64(a2, a3);
            v[q] = C(_mm_srai_epi32::<2>(e0), _mm_srai_epi32::<2>(e2));
            v[7 - q] = C(rev(_mm_srai_epi32::<2>(e3)), rev(_mm_srai_epi32::<2>(e1)));
        }
        let v = fft_32_regs(v);
        let tp = tmp.as_mut_ptr();
        for (q, c) in v.iter().enumerate() {
            st_at(tp, 4 * q, *c);
        }
        // (a[1], a[2], a[3], b[0])
        let ext1 = |a: C, b: C| C(_mm_alignr_epi8::<4>(b.0, a.0), _mm_alignr_epi8::<4>(b.1, a.1));
        let (x, t) = (p_dat.as_mut_ptr(), tab.as_ptr());
        sse2::dct_ii_block_regs(ext1(v[0], v[1]), v[7], x, L, M, 1, t);
        sse2::dct_ii_block_regs(ext1(v[1], v[2]), v[6], x, L, M, 5, t);
        sse2::dct_ii_block_regs(ext1(v[2], v[3]), v[5], x, L, M, 9, t);
        sse2::dct_ii_block_regs(v[3], ext1(v[4], v[5]), x, L, M, 12, t);
        (
            _mm_cvtsi128_si32(v[0].0),
            _mm_cvtsi128_si32(v[0].1),
            _mm_cvtsi128_si32(v[4].0),
            _mm_cvtsi128_si32(v[4].1),
        )
    };
    *p_dat_e += crate::fft::SCALEFACTOR32;
    // the single values (complex 0 and m / 2 of tmp)
    let w = tw((M / 2) * inc);
    let (accu1, accu2) = cplx_mult(tm0, tm1, w.re, w.im);
    p_dat[L - (M / 2)] = accu2;
    p_dat[M / 2] = accu1;

    p_dat[0] = t0.wrapping_add(t1);
    p_dat[M] = f_mult(t0.wrapping_sub(t1), tw(M * inc).re); // cos((PI/(2*L))*M);

    *p_dat_e += 2;
    true
}

/// The x86 form of [`dct_iv_dst_iv_neon_n`] for the variants other than `aarch64` (selected at
/// run time with AVX2; the 128-bit blocks of `x86::lane4` and the vector FFT of
/// `crate::fft::x86_16`): the same pre twiddling, FFT and post twiddling as the general code for
/// these lengths, with the values kept in registers between the steps.
///
/// # Safety
///
/// AVX2 must be available.
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[target_feature(enable = "avx2")]
unsafe fn dct_iv_dst_iv_x86_n<const DST: bool, const L: usize>(
    p_dat: &mut [FixpDbl],
    p_dat_e: &mut i32,
) {
    use crate::fft::x86_16::{fft_16_regs, fft_32_regs};
    #[cfg(target_arch = "x86")]
    use core::arch::x86::_mm_setzero_si128;
    #[cfg(target_arch = "x86_64")]
    use core::arch::x86_64::_mm_setzero_si128;
    use x86::lane4::{post_regs, pre_regs, C};
    const { assert!(L == 32 || L == 64) };
    let (twiddle, sin_twiddle, sin_step) = dct_get_tables(L as i32);
    let (Some(twiddle), Some(sin_twiddle)) = (twiddle, sin_twiddle) else { return };
    let sin_step = sin_step as usize;
    let mu = L / 2;
    let k_end = mu / 2; // (mu + 1) >> 1, mu even
    let nv = L / 8; // complex vectors of 4 values
    let x: &mut [FixpDbl; L] = (&mut p_dat[..L]).try_into().unwrap();
    // the blocks read twiddle[0 .. mu] and sin_twiddle[j * sin_step] for j < k_end
    assert!(twiddle.len() >= mu && sin_twiddle.len() > (k_end - 1) * sin_step);
    let (tw, st) = (twiddle.as_ptr(), sin_twiddle.as_ptr());
    let xp = x.as_mut_ptr();
    // SAFETY: AVX2 (and so SSE4.1) is available; the pre blocks at 2 it (it = 4b < mu / 2) and
    // the post blocks at j = 4b < k_end stay inside x and the tables.
    unsafe {
        let z = _mm_setzero_si128();
        let mut v = [C(z, z); 8];
        for b in 0..nv / 2 {
            let (f, bk) = pre_regs::<DST>(xp, L, 8 * b, tw);
            v[b] = f;
            v[nv - 1 - b] = bk;
        }
        if L == 32 {
            let r = fft_16_regs([v[0], v[1], v[2], v[3]]);
            v[..4].copy_from_slice(&r);
            *p_dat_e += crate::fft::SCALEFACTOR16;
        } else {
            v = fft_32_regs(v);
            *p_dat_e += crate::fft::SCALEFACTOR32;
        }
        post_regs::<DST, true, false>(v[0], v[nv - 1], xp, L, 0, st, sin_step);
        for b in 1..nv / 2 - 1 {
            post_regs::<DST, false, false>(v[b], v[nv - 1 - b], xp, L, 4 * b, st, sin_step);
        }
        let b = nv / 2 - 1;
        post_regs::<DST, false, true>(v[b], v[nv - 1 - b], xp, L, 4 * b, st, sin_step);
    }
    // Add twiddeling scale.
    *p_dat_e += 2;
}

/// `dct_IV(pDat, L, pDat_e)`: DCT-IV of length `L`, the straight port (see [`dct_iv`]).
pub fn dct_iv_ref(p_dat: &mut [FixpDbl], l: i32, p_dat_e: &mut i32) {
    let m = l >> 1;

    fdk_assert!(l >= 4);

    let (twiddle, sin_twiddle, sin_step) = dct_get_tables(l);
    let (Some(twiddle), Some(sin_twiddle)) = (twiddle, sin_twiddle) else { return };

    let (l, mu, sin_step) = (l as usize, m as usize, sin_step as usize);

    {
        let mut p_dat_0 = 0usize;
        let mut p_dat_1 = l - 2;
        let mut i = 0usize;

        // 29 cycles on ARM926
        while (i as i32) < m - 1 {
            let mut accu1 = p_dat[p_dat_1 + 1];
            let mut accu2 = p_dat[p_dat_0];
            let mut accu3 = p_dat[p_dat_0 + 1];
            let mut accu4 = p_dat[p_dat_1];

            (accu1, accu2) = cplx_mult_div2(accu1, accu2, twiddle[i].re, twiddle[i].im);
            (accu3, accu4) = cplx_mult_div2(accu4, accu3, twiddle[i + 1].re, twiddle[i + 1].im);

            p_dat[p_dat_0] = accu2 >> 1;
            p_dat[p_dat_0 + 1] = accu1 >> 1;
            p_dat[p_dat_1] = accu4 >> 1;
            p_dat[p_dat_1 + 1] = (accu3 >> 1).wrapping_neg();

            i += 2;
            p_dat_0 += 2;
            p_dat_1 -= 2;
        }
        if mu & 1 != 0 {
            let mut accu1 = p_dat[p_dat_1 + 1];
            let mut accu2 = p_dat[p_dat_0];

            (accu1, accu2) = cplx_mult_div2(accu1, accu2, twiddle[i].re, twiddle[i].im);

            p_dat[p_dat_0] = accu2 >> 1;
            p_dat[p_dat_0 + 1] = accu1 >> 1;
        }
    }

    crate::fft::fft_ref(m, p_dat, p_dat_e);

    {
        let mut p_dat_0 = 0usize;
        let mut p_dat_1 = l - 2;

        // Sin and Cos values are 0.0f and 1.0f
        let mut accu1 = p_dat[p_dat_1];
        let mut accu2 = p_dat[p_dat_1 + 1];

        p_dat[p_dat_1 + 1] = p_dat[p_dat_0 + 1].wrapping_neg();

        // 28 cycles for ARM926
        let mut idx = sin_step;
        for _ in 1..((mu + 1) >> 1) {
            let twd = sin_twiddle[idx];
            let (accu3, accu4) = cplx_mult(accu1, accu2, twd.re, twd.im);
            p_dat[p_dat_0 + 1] = accu3;
            p_dat[p_dat_1] = accu4;

            p_dat_0 += 2;
            p_dat_1 -= 2;

            let (accu3, accu4) = cplx_mult(p_dat[p_dat_0 + 1], p_dat[p_dat_0], twd.re, twd.im);

            accu1 = p_dat[p_dat_1];
            accu2 = p_dat[p_dat_1 + 1];

            p_dat[p_dat_1 + 1] = accu3.wrapping_neg();
            p_dat[p_dat_0] = accu4;
            idx += sin_step;
        }

        if (mu & 1) == 0 {
            // Last Sin and Cos value pair are the same
            accu1 = f_mult(accu1, W_PI_FOURTH);
            accu2 = f_mult(accu2, W_PI_FOURTH);

            p_dat[p_dat_1] = accu1.wrapping_add(accu2);
            p_dat[p_dat_0 + 1] = accu1.wrapping_sub(accu2);
        }
    }

    // Add twiddeling scale.
    *p_dat_e += 2;
}

/// `dst_IV(pDat, L, pDat_e)`: DST-IV of length `L`, the straight port (see [`dst_iv`]).
pub fn dst_iv_ref(p_dat: &mut [FixpDbl], l: i32, p_dat_e: &mut i32) {
    let m = l >> 1;

    fdk_assert!(l >= 4);

    let (twiddle, sin_twiddle, sin_step) = dct_get_tables(l);
    let (Some(twiddle), Some(sin_twiddle)) = (twiddle, sin_twiddle) else { return };

    let (l, mu, sin_step) = (l as usize, m as usize, sin_step as usize);

    {
        let mut p_dat_0 = 0usize;
        let mut p_dat_1 = l - 2;
        let mut i = 0usize;

        // 34 cycles on ARM926
        while (i as i32) < m - 1 {
            let mut accu1 = p_dat[p_dat_1 + 1] >> 1;
            let mut accu2 = (p_dat[p_dat_0] >> 1).wrapping_neg();
            let mut accu3 = p_dat[p_dat_0 + 1] >> 1;
            let mut accu4 = (p_dat[p_dat_1] >> 1).wrapping_neg();

            (accu1, accu2) = cplx_mult_div2(accu1, accu2, twiddle[i].re, twiddle[i].im);
            (accu3, accu4) = cplx_mult_div2(accu4, accu3, twiddle[i + 1].re, twiddle[i + 1].im);

            p_dat[p_dat_0] = accu2;
            p_dat[p_dat_0 + 1] = accu1;
            p_dat[p_dat_1] = accu4;
            p_dat[p_dat_1 + 1] = accu3.wrapping_neg();

            i += 2;
            p_dat_0 += 2;
            p_dat_1 -= 2;
        }
        if mu & 1 != 0 {
            let mut accu1 = p_dat[p_dat_1 + 1];
            let mut accu2 = p_dat[p_dat_0].wrapping_neg();

            (accu1, accu2) = cplx_mult_div2(accu1, accu2, twiddle[i].re, twiddle[i].im);

            p_dat[p_dat_0] = accu2 >> 1;
            p_dat[p_dat_0 + 1] = accu1 >> 1;
        }
    }

    crate::fft::fft_ref(m, p_dat, p_dat_e);

    {
        let mut p_dat_0 = 0usize;
        let mut p_dat_1 = l - 2;

        // Sin and Cos values are 0.0f and 1.0f
        let mut accu1 = p_dat[p_dat_1];
        let mut accu2 = p_dat[p_dat_1 + 1];

        p_dat[p_dat_1 + 1] = p_dat[p_dat_0].wrapping_neg();
        p_dat[p_dat_0] = p_dat[p_dat_0 + 1];

        let mut idx = sin_step;
        for _ in 1..((mu + 1) >> 1) {
            let twd = sin_twiddle[idx];

            let (accu3, accu4) = cplx_mult(accu1, accu2, twd.re, twd.im);
            p_dat[p_dat_1] = accu3.wrapping_neg();
            p_dat[p_dat_0 + 1] = accu4.wrapping_neg();

            p_dat_0 += 2;
            p_dat_1 -= 2;

            let (accu3, accu4) = cplx_mult(p_dat[p_dat_0 + 1], p_dat[p_dat_0], twd.re, twd.im);

            accu1 = p_dat[p_dat_1];
            accu2 = p_dat[p_dat_1 + 1];

            p_dat[p_dat_0] = accu3;
            p_dat[p_dat_1 + 1] = accu4.wrapping_neg();
            idx += sin_step;
        }

        if (mu & 1) == 0 {
            // Last Sin and Cos value pair are the same
            accu1 = f_mult(accu1, W_PI_FOURTH);
            accu2 = f_mult(accu2, W_PI_FOURTH);

            p_dat[p_dat_0 + 1] = accu1.wrapping_neg().wrapping_sub(accu2);
            p_dat[p_dat_1] = accu2.wrapping_sub(accu1);
        }
    }

    // Add twiddeling scale.
    *p_dat_e += 2;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fft::testutil::*;

    /// The transform lengths with an FFT and tables.
    fn lengths(type4: bool) -> Vec<i32> {
        (4..=1024)
            .filter(|&l| {
                let (twiddle, sin_twiddle, _) = dct_get_tables(l);
                l % 2 == 0
                    && sin_twiddle.is_some()
                    && (!type4 || twiddle.is_some())
                    && [
                        2, 3, 4, 5, 6, 8, 10, 12, 15, 16, 20, 24, 32, 48, 60, 64, 80, 96, 120, 128,
                        192, 240, 256, 384, 480, 512,
                    ]
                    .contains(&(l / 2))
            })
            .collect()
    }

    fn cases(l: usize) -> usize {
        if l <= 128 {
            1500
        } else {
            200
        }
    }

    /// dct_IV, dst_IV against the straight ports (with the arithmetic of the selected variant).
    #[test]
    fn dct_iv_dst_iv_equal_straight_port() {
        type F = fn(&mut [i32], i32, &mut i32);
        let fs: [(F, F); 2] = [(dct_iv, dct_iv_ref), (dst_iv, dst_iv_ref)];
        let mut r = Rng(0x5eed_0002);
        for l in lengths(true) {
            for (fast, straight) in fs {
                for k in 0..cases(l as usize) {
                    let x = signal(&mut r, k, l as usize);
                    let (mut a, mut b) = (x.clone(), x.clone());
                    let (mut ea, mut eb) = (3, 3);
                    fast(&mut a, l, &mut ea);
                    straight(&mut b, l, &mut eb);
                    assert_eq!((a, ea), (b, eb), "L {l} case {k}");
                }
            }
        }
    }

    /// dct_II, dct_III, dst_III against the straight ports.
    #[test]
    fn dct_ii_iii_equal_straight_port() {
        type F = fn(&mut [i32], &mut [i32], i32, &mut i32);
        let fs: [(F, F); 3] =
            [(dct_ii, dct_ii_ref), (dct_iii, dct_iii_ref), (dst_iii, dst_iii_ref)];
        let mut r = Rng(0x5eed_0003);
        for l in lengths(false).into_iter().filter(|l| l % 4 == 0) {
            for (fast, straight) in fs {
                for k in 0..cases(l as usize) {
                    let x = signal(&mut r, k, l as usize);
                    let t: Vec<i32> = (0..l).map(|_| r.i32()).collect();
                    let (mut a, mut b) = (x.clone(), x.clone());
                    let (mut ta, mut tb) = (t.clone(), t);
                    let (mut ea, mut eb) = (3, 3);
                    fast(&mut a, &mut ta, l, &mut ea);
                    straight(&mut b, &mut tb, l, &mut eb);
                    assert_eq!((a, ta, ea), (b, tb, eb), "L {l} case {k}");
                }
            }
        }
    }

    #[test]
    fn twiddle_index_closed_form() {
        for m in 2..1024 {
            for inc in [1, 2, 3, 4, 8] {
                let idx = dct_twiddle_indices(m, inc);
                for i in 1..m / 2 {
                    assert_eq!(dct_twiddle_index(m, inc, i), idx[i], "m {m} inc {inc} i {i}");
                }
            }
        }
    }
}
