// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libFDK/include/fft_rad2.h, libFDK/src/fft_rad2.cpp, libFDK/src/arm/fft_rad2_arm.cpp.

//! Port of `fft_rad2.h`: in-place radix-2 decimation-in-time FFT.
//!
//! The `aarch64` build compiles `arm/fft_rad2_arm.cpp` (included by `fft_rad2.cpp`), which
//! replaces the generic `dit_fft` (`x86` and `generic`). The two implementations walk the data in
//! a different order (pointers and `do ... while` loops instead of indices and `for` loops), but
//! perform the same operations on the same elements in the same order for `ldn >= 3`; they differ
//! in their arithmetic only through `cplxMultDiv2` (see [`crate::cplx_mul`]). Both are ported:
//! [`dit_fft_arm`] and [`dit_fft_generic`].
//!
//! The additions overflow for full-scale input (the input needs one bit of headroom; undefined
//! behavior in C); the port wraps, which the `cref` tests with full-scale input confirm for both
//! reference builds.

use crate::archdef::{stc, FixpStp};
use crate::common_fix::*;
use crate::scramble::scramble;

/// `dit_fft(x, ldn, trigdata, trigDataSize)`: in-place complex FFT of length `2^ldn` on
/// interleaved real/imaginary values, with the bit reversal at the start.
///
/// The input must have one bit of headroom. `trigdata` is a sine table of at least
/// `2^ldn / 2` values (`trigDataSize` entries).
///
/// On `aarch64` targets the `aarch64` variant runs a NEON implementation for `SineTable512` with
/// `trigDataSize` 512 (all calls of the library), which computes every butterfly with exactly the
/// arithmetic of [`dit_fft_arm`] (see `neon`). The straight ports [`dit_fft_arm`] and
/// [`dit_fft_generic`] remain for the other parameters and variants.
///
/// Elsewhere (the other variants, other targets, other tables) [`dit_fft_scalar`] runs: the
/// loops of [`dit_fft_arm`] with unchecked indexing (both straight ports perform the same
/// operations on the same elements in the same order for `ldn >= 3`, see the module
/// documentation).
pub fn dit_fft(x: &mut [FixpDbl], ldn: i32, trigdata: &[FixpStp], trig_data_size: i32) {
    #[cfg(target_arch = "aarch64")]
    if ARCH_ARM && neon_supported(ldn, trigdata, trig_data_size) {
        neon::dit_fft(x, ldn);
        return;
    }
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    if !ARCH_ARM
        && (x86::MIN_LDN..=x86::MAX_LDN).contains(&ldn)
        && trig_data_size == 512
        && core::ptr::eq(trigdata.as_ptr(), crate::tools_rom::SINE_TABLE_512.as_ptr())
        && x86::avx2()
    {
        // SAFETY: AVX2 is available.
        unsafe { x86::dit_fft(x, ldn) };
        return;
    }
    if (3..=16).contains(&ldn) && x.len() >= 2 << ldn {
        dit_fft_scalar(x, ldn, trigdata, trig_data_size)
    } else if ARCH_ARM {
        dit_fft_arm(x, ldn, trigdata, trig_data_size)
    } else {
        dit_fft_generic(x, ldn, trigdata, trig_data_size)
    }
}

/// [`dit_fft_arm`] with unchecked indexing of `x`, with the `cplxMultDiv2` of the selected
/// variant: the same values as [`dit_fft_arm`] (`aarch64`) and [`dit_fft_generic`] (the other
/// variants) for `3 <= ldn <= 16`.
///
/// Panics if `ldn` is outside of `3..=16` or `x` holds fewer than `2 * 2^ldn` values.
pub fn dit_fft_scalar(x: &mut [FixpDbl], ldn: i32, trigdata: &[FixpStp], trig_data_size: i32) {
    assert!((3..=16).contains(&ldn));
    let n: usize = 1 << ldn;
    let x = &mut x[..2 * n];
    let p = x.as_mut_ptr();
    // SAFETY (all accesses below): every index is below 2 n, the length of `x`: the bit
    // reversal swaps the complex values m, j < n; the radix-4 pass reads and writes the groups
    // i .. i + 8 < 2 n; in a stage with the span `mh` (complex values, mh < n) the butterflies
    // access the complex values xt1 / 2 and xt2 / 2 + 1 <= 2 mh + (n - 2 mh) + ... < n, as in
    // the straight port, whose loops this is (with the same bounds).
    macro_rules! x {
        ($i:expr) => {
            *p.add($i)
        };
    }
    unsafe {
        // scramble(x, n)
        let mut j: usize = 0;
        let mut m: usize = 1;
        while m < n - 1 {
            let mut k = n >> 1;
            loop {
                j ^= k;
                if (j & k) != 0 {
                    break;
                }
                k >>= 1;
            }
            if j > m {
                core::ptr::swap(p.add(2 * m), p.add(2 * j));
                core::ptr::swap(p.add(2 * m + 1), p.add(2 * j + 1));
            }
            m += 1;
        }

        // 1+2 stage radix 4
        let mut i = 0usize;
        while i < 2 * n {
            let mut a00 = x!(i).wrapping_add(x!(i + 2)) >> 1;
            let mut a10 = x!(i + 4).wrapping_add(x!(i + 6)) >> 1;
            let mut a20 = x!(i + 1).wrapping_add(x!(i + 3)) >> 1;
            let mut a30 = x!(i + 5).wrapping_add(x!(i + 7)) >> 1;

            x!(i) = a00.wrapping_add(a10);
            x!(i + 4) = a00.wrapping_sub(a10);
            x!(i + 1) = a20.wrapping_add(a30);
            x!(i + 5) = a20.wrapping_sub(a30);

            a00 = a00.wrapping_sub(x!(i + 2));
            a10 = a10.wrapping_sub(x!(i + 6));
            a20 = a20.wrapping_sub(x!(i + 3));
            a30 = a30.wrapping_sub(x!(i + 7));

            x!(i + 2) = a00.wrapping_add(a30);
            x!(i + 6) = a00.wrapping_sub(a30);
            x!(i + 3) = a20.wrapping_sub(a10);
            x!(i + 7) = a20.wrapping_add(a10);
            i += 8;
        }

        let mut mh: usize = 1 << 1;
        let mut ldm: i32 = ldn - 2;
        let mut trigstep: i32 = trig_data_size;

        loop {
            let mut p_trig_data: usize = 0;

            mh <<= 1;
            trigstep >>= 1;

            fdk_assert!(trigstep > 0);

            {
                let mut xt1: usize = 0;
                let mut r = n;
                loop {
                    let mut xt2 = xt1 + (mh << 1);

                    let mut vi = half(x!(xt2 + 1));
                    let mut vr = half(x!(xt2));
                    let mut ur = half(x!(xt1));
                    let mut ui = half(x!(xt1 + 1));
                    x!(xt1) = ur.wrapping_add(vr);
                    x!(xt1 + 1) = ui.wrapping_add(vi);
                    x!(xt2) = ur.wrapping_sub(vr);
                    x!(xt2 + 1) = ui.wrapping_sub(vi);

                    xt1 += mh;
                    xt2 += mh;

                    vr = half(x!(xt2 + 1));
                    vi = half(x!(xt2));
                    ur = half(x!(xt1));
                    ui = half(x!(xt1 + 1));
                    x!(xt1) = ur.wrapping_add(vr);
                    x!(xt1 + 1) = ui.wrapping_sub(vi);
                    x!(xt2) = ur.wrapping_sub(vr);
                    x!(xt2 + 1) = ui.wrapping_add(vi);

                    xt1 = xt2 + mh;
                    r -= mh << 1;
                    if r == 0 {
                        break;
                    }
                }
            }
            let mut j: usize = 4;
            while j < mh {
                let mut xt1 = j >> 1;
                let mut r = n;

                p_trig_data += trigstep as usize;
                let cs = trigdata[p_trig_data];

                loop {
                    let mut xt2 = xt1 + (mh << 1);

                    let (mut vi, mut vr) = cplx_mult_div2(x!(xt2 + 1), x!(xt2), cs.re, cs.im);
                    let mut ur = half(x!(xt1));
                    let mut ui = half(x!(xt1 + 1));
                    x!(xt1) = ur.wrapping_add(vr);
                    x!(xt1 + 1) = ui.wrapping_add(vi);
                    x!(xt2) = ur.wrapping_sub(vr);
                    x!(xt2 + 1) = ui.wrapping_sub(vi);

                    xt1 += mh;
                    xt2 += mh;

                    (vr, vi) = cplx_mult_div2(x!(xt2 + 1), x!(xt2), cs.re, cs.im);
                    ur = half(x!(xt1));
                    ui = half(x!(xt1 + 1));
                    x!(xt1) = ur.wrapping_add(vr);
                    x!(xt1 + 1) = ui.wrapping_sub(vi);
                    x!(xt2) = ur.wrapping_sub(vr);
                    x!(xt2 + 1) = ui.wrapping_add(vi);

                    // Same as above but for t1,t2 with j>mh/4 and thus cs swapped
                    xt1 -= j;
                    xt2 = xt1 + (mh << 1);

                    (vi, vr) = cplx_mult_div2(x!(xt2), x!(xt2 + 1), cs.re, cs.im);
                    ur = half(x!(xt1));
                    ui = half(x!(xt1 + 1));
                    x!(xt1) = ur.wrapping_add(vr);
                    x!(xt1 + 1) = ui.wrapping_sub(vi);
                    x!(xt2) = ur.wrapping_sub(vr);
                    x!(xt2 + 1) = ui.wrapping_add(vi);

                    xt1 += mh;
                    xt2 += mh;

                    (vr, vi) = cplx_mult_div2(x!(xt2), x!(xt2 + 1), cs.re, cs.im);
                    ur = half(x!(xt1));
                    ui = half(x!(xt1 + 1));
                    x!(xt1) = ur.wrapping_sub(vr);
                    x!(xt1 + 1) = ui.wrapping_sub(vi);
                    x!(xt2) = ur.wrapping_add(vr);
                    x!(xt2 + 1) = ui.wrapping_add(vi);

                    xt1 = xt2 + j;
                    r -= mh << 1;
                    if r == 0 {
                        break;
                    }
                }
                j += 4;
            }
            {
                let mut xt1 = mh >> 1;
                let mut r = n;
                let w = stc(0x5a82799a);

                loop {
                    let mut xt2 = xt1 + (mh << 1);

                    let (mut vi, mut vr) = cplx_mult_div2(x!(xt2 + 1), x!(xt2), w, w);
                    let mut ur = half(x!(xt1));
                    let mut ui = half(x!(xt1 + 1));
                    x!(xt1) = ur.wrapping_add(vr);
                    x!(xt1 + 1) = ui.wrapping_add(vi);
                    x!(xt2) = ur.wrapping_sub(vr);
                    x!(xt2 + 1) = ui.wrapping_sub(vi);

                    xt1 += mh;
                    xt2 += mh;

                    (vr, vi) = cplx_mult_div2(x!(xt2 + 1), x!(xt2), w, w);
                    ur = half(x!(xt1));
                    ui = half(x!(xt1 + 1));
                    x!(xt1) = ur.wrapping_add(vr);
                    x!(xt1 + 1) = ui.wrapping_sub(vi);
                    x!(xt2) = ur.wrapping_sub(vr);
                    x!(xt2 + 1) = ui.wrapping_add(vi);

                    xt1 = xt2 + mh;
                    r -= mh << 1;
                    if r == 0 {
                        break;
                    }
                }
            }
            ldm -= 1;
            if ldm == 0 {
                break;
            }
        }
    }
}

#[cfg(target_arch = "aarch64")]
mod neon;
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
pub(crate) mod x86;

/// Whether the NEON implementation handles these parameters: `SineTable512` (the table itself,
/// not a copy) with `trigDataSize` 512 and `4 <= ldn <= 10`.
#[cfg(target_arch = "aarch64")]
#[inline(always)]
fn neon_supported(ldn: i32, trigdata: &[FixpStp], trig_data_size: i32) -> bool {
    (neon::MIN_LDN..=neon::MAX_LDN).contains(&ldn)
        && trig_data_size == 512
        && core::ptr::eq(trigdata.as_ptr(), crate::tools_rom::SINE_TABLE_512.as_ptr())
}

/// The NEON implementation of [`dit_fft`] (`aarch64` arithmetic) for `SineTable512`, without the
/// fallback; for tests and benchmarks. Panics for `ldn` outside of `4..=10`.
#[cfg(target_arch = "aarch64")]
pub fn dit_fft_neon(x: &mut [FixpDbl], ldn: i32) {
    neon::dit_fft(x, ldn)
}

/// The first two stages (radix 4) of both implementations.
#[inline(always)]
fn radix4_first_stages(x: &mut [FixpDbl], n: i32) {
    let mut i = 0usize;
    while (i as i32) < n * 2 {
        let mut a00 = x[i].wrapping_add(x[i + 2]) >> 1; // Re A + Re B
        let mut a10 = x[i + 4].wrapping_add(x[i + 6]) >> 1; // Re C + Re D
        let mut a20 = x[i + 1].wrapping_add(x[i + 3]) >> 1; // Im A + Im B
        let mut a30 = x[i + 5].wrapping_add(x[i + 7]) >> 1; // Im C + Im D

        x[i] = a00.wrapping_add(a10); // Re A' = Re A + Re B + Re C + Re D
        x[i + 4] = a00.wrapping_sub(a10); // Re C' = Re A + Re B - Re C - Re D
        x[i + 1] = a20.wrapping_add(a30); // Im A' = Im A + Im B + Im C + Im D
        x[i + 5] = a20.wrapping_sub(a30); // Im C' = Im A + Im B - Im C - Im D

        a00 = a00.wrapping_sub(x[i + 2]); // Re A - Re B
        a10 = a10.wrapping_sub(x[i + 6]); // Re C - Re D
        a20 = a20.wrapping_sub(x[i + 3]); // Im A - Im B
        a30 = a30.wrapping_sub(x[i + 7]); // Im C - Im D

        x[i + 2] = a00.wrapping_add(a30); // Re B' = Re A - Re B + Im C - Im D
        x[i + 6] = a00.wrapping_sub(a30); // Re D' = Re A - Re B - Im C + Im D
        x[i + 3] = a20.wrapping_sub(a10); // Im B' = Im A - Im B - Re C + Re D
        x[i + 7] = a20.wrapping_add(a10); // Im D' = Im A - Im B + Re C - Re D
        i += 8;
    }
}

/// `v >> 1`
#[inline(always)]
fn half(v: FixpDbl) -> FixpDbl {
    v >> 1
}

/// `dit_fft` of `libFDK/src/fft_rad2.cpp` (`x86` and `generic`).
pub fn dit_fft_generic(x: &mut [FixpDbl], ldn: i32, trigdata: &[FixpStp], trig_data_size: i32) {
    let n: i32 = 1 << ldn;

    scramble(x, n);

    // 1+2 stage radix 4
    radix4_first_stages(x, n);

    for ldm in 3..=ldn {
        let m: i32 = 1 << ldm;
        let mh: i32 = m >> 1;

        let trigstep: i32 = (trig_data_size << 2) >> ldm;

        fdk_assert!(trigstep > 0);

        // Do first iteration with c=1.0 and s=0.0 separately to avoid loosing to much
        // precision. Beware: The impact on the overal FFT precision is rather large.
        {
            // block 1
            let j = 0;
            let mut r = 0;
            while r < n {
                let mut t1 = ((r + j) << 1) as usize;
                let mut t2 = t1 + (mh << 1) as usize;

                // cplxMultDiv2(&vi, &vr, x[t2+1], x[t2], (FIXP_SGL)1.0, (FIXP_SGL)0.0);
                let mut vi = half(x[t2 + 1]);
                let mut vr = half(x[t2]);

                let mut ur = half(x[t1]);
                let mut ui = half(x[t1 + 1]);

                x[t1] = ur.wrapping_add(vr);
                x[t1 + 1] = ui.wrapping_add(vi);

                x[t2] = ur.wrapping_sub(vr);
                x[t2 + 1] = ui.wrapping_sub(vi);

                t1 += mh as usize;
                t2 = t1 + (mh << 1) as usize;

                // cplxMultDiv2(&vr, &vi, x[t2+1], x[t2], (FIXP_SGL)1.0, (FIXP_SGL)0.0);
                vr = half(x[t2 + 1]);
                vi = half(x[t2]);

                ur = half(x[t1]);
                ui = half(x[t1 + 1]);

                x[t1] = ur.wrapping_add(vr);
                x[t1 + 1] = ui.wrapping_sub(vi);

                x[t2] = ur.wrapping_sub(vr);
                x[t2 + 1] = ui.wrapping_add(vi);

                r += m;
            }
        } // end of  block 1

        for j in 1..mh / 4 {
            let cs = trigdata[(j * trigstep) as usize];

            let mut r = 0;
            while r < n {
                let mut t1 = ((r + j) << 1) as usize;
                let mut t2 = t1 + (mh << 1) as usize;

                let (mut vi, mut vr) = cplx_mult_div2(x[t2 + 1], x[t2], cs.re, cs.im);

                let mut ur = half(x[t1]);
                let mut ui = half(x[t1 + 1]);

                x[t1] = ur.wrapping_add(vr);
                x[t1 + 1] = ui.wrapping_add(vi);

                x[t2] = ur.wrapping_sub(vr);
                x[t2 + 1] = ui.wrapping_sub(vi);

                t1 += mh as usize;
                t2 = t1 + (mh << 1) as usize;

                (vr, vi) = cplx_mult_div2(x[t2 + 1], x[t2], cs.re, cs.im);

                ur = half(x[t1]);
                ui = half(x[t1 + 1]);

                x[t1] = ur.wrapping_add(vr);
                x[t1 + 1] = ui.wrapping_sub(vi);

                x[t2] = ur.wrapping_sub(vr);
                x[t2 + 1] = ui.wrapping_add(vi);

                // Same as above but for t1,t2 with j>mh/4 and thus cs swapped
                t1 = ((r + mh / 2 - j) << 1) as usize;
                t2 = t1 + (mh << 1) as usize;

                (vi, vr) = cplx_mult_div2(x[t2], x[t2 + 1], cs.re, cs.im);

                ur = half(x[t1]);
                ui = half(x[t1 + 1]);

                x[t1] = ur.wrapping_add(vr);
                x[t1 + 1] = ui.wrapping_sub(vi);

                x[t2] = ur.wrapping_sub(vr);
                x[t2 + 1] = ui.wrapping_add(vi);

                t1 += mh as usize;
                t2 = t1 + (mh << 1) as usize;

                (vr, vi) = cplx_mult_div2(x[t2], x[t2 + 1], cs.re, cs.im);

                ur = half(x[t1]);
                ui = half(x[t1 + 1]);

                x[t1] = ur.wrapping_sub(vr);
                x[t1 + 1] = ui.wrapping_sub(vi);

                x[t2] = ur.wrapping_add(vr);
                x[t2 + 1] = ui.wrapping_add(vi);

                r += m;
            }
        }

        {
            // block 2
            let j = mh / 4;
            let w = stc(0x5a82799a);

            let mut r = 0;
            while r < n {
                let mut t1 = ((r + j) << 1) as usize;
                let mut t2 = t1 + (mh << 1) as usize;

                let (mut vi, mut vr) = cplx_mult_div2(x[t2 + 1], x[t2], w, w);

                let mut ur = half(x[t1]);
                let mut ui = half(x[t1 + 1]);

                x[t1] = ur.wrapping_add(vr);
                x[t1 + 1] = ui.wrapping_add(vi);

                x[t2] = ur.wrapping_sub(vr);
                x[t2 + 1] = ui.wrapping_sub(vi);

                t1 += mh as usize;
                t2 = t1 + (mh << 1) as usize;

                (vr, vi) = cplx_mult_div2(x[t2 + 1], x[t2], w, w);

                ur = half(x[t1]);
                ui = half(x[t1 + 1]);

                x[t1] = ur.wrapping_add(vr);
                x[t1 + 1] = ui.wrapping_sub(vi);

                x[t2] = ur.wrapping_sub(vr);
                x[t2 + 1] = ui.wrapping_add(vi);

                r += m;
            }
        } // end of block 2
    }
}

/// `dit_fft` of `libFDK/src/arm/fft_rad2_arm.cpp` (`aarch64`).
///
/// The C pointers `xt1`, `xt2` and `pTrigData` become indices.
///
/// Deviation: for `ldn < 3` the C stage loop has no defined result; the port returns after the
/// radix-4 stages.
pub fn dit_fft_arm(x: &mut [FixpDbl], ldn: i32, trigdata: &[FixpStp], trig_data_size: i32) {
    let n: i32 = 1 << ldn;

    scramble(x, n);

    // 1+2 stage radix 4
    radix4_first_stages(x, n);

    fdk_assert!(ldn >= 3);
    if ldn < 3 {
        return;
    }

    let mut mh: i32 = 1 << 1;
    let mut ldm: i32 = ldn - 2;
    let mut trigstep: i32 = trig_data_size;

    loop {
        let mut p_trig_data: usize = 0;

        mh <<= 1;
        trigstep >>= 1;

        fdk_assert!(trigstep > 0);

        // Do first iteration with c=1.0 and s=0.0 separately to avoid loosing to much
        // precision. Beware: The impact on the overal FFT precision is rather large.
        {
            let mut xt1: usize = 0;
            let mut r: i32 = n;

            loop {
                let mut xt2 = xt1 + (mh << 1) as usize;

                // cplxMultDiv2(&vi, &vr, x[t2+1], x[t2], (FIXP_SGL)1.0, (FIXP_SGL)0.0);
                let mut vi = half(x[xt2 + 1]);
                let mut vr = half(x[xt2]);

                let mut ur = half(x[xt1]);
                let mut ui = half(x[xt1 + 1]);

                x[xt1] = ur.wrapping_add(vr);
                x[xt1 + 1] = ui.wrapping_add(vi);

                x[xt2] = ur.wrapping_sub(vr);
                x[xt2 + 1] = ui.wrapping_sub(vi);

                xt1 += mh as usize;
                xt2 += mh as usize;

                // cplxMultDiv2(&vr, &vi, x[t2+1], x[t2], (FIXP_SGL)1.0, (FIXP_SGL)0.0);
                vr = half(x[xt2 + 1]);
                vi = half(x[xt2]);

                ur = half(x[xt1]);
                ui = half(x[xt1 + 1]);

                x[xt1] = ur.wrapping_add(vr);
                x[xt1 + 1] = ui.wrapping_sub(vi);

                x[xt2] = ur.wrapping_sub(vr);
                x[xt2 + 1] = ui.wrapping_add(vi);

                xt1 = xt2 + mh as usize;

                r -= mh << 1;
                if r == 0 {
                    break;
                }
            }
        }
        let mut j: i32 = 4;
        while j < mh {
            let mut xt1 = (j >> 1) as usize;
            let mut r: i32 = n;

            p_trig_data += trigstep as usize;
            let cs = trigdata[p_trig_data];

            loop {
                let mut xt2 = xt1 + (mh << 1) as usize;

                let (mut vi, mut vr) = cplx_mult_div2(x[xt2 + 1], x[xt2], cs.re, cs.im);

                let mut ur = half(x[xt1]);
                let mut ui = half(x[xt1 + 1]);

                x[xt1] = ur.wrapping_add(vr);
                x[xt1 + 1] = ui.wrapping_add(vi);

                x[xt2] = ur.wrapping_sub(vr);
                x[xt2 + 1] = ui.wrapping_sub(vi);

                xt1 += mh as usize;
                xt2 += mh as usize;

                (vr, vi) = cplx_mult_div2(x[xt2 + 1], x[xt2], cs.re, cs.im);

                ur = half(x[xt1]);
                ui = half(x[xt1 + 1]);

                x[xt1] = ur.wrapping_add(vr);
                x[xt1 + 1] = ui.wrapping_sub(vi);

                x[xt2] = ur.wrapping_sub(vr);
                x[xt2 + 1] = ui.wrapping_add(vi);

                // Same as above but for t1,t2 with j>mh/4 and thus cs swapped
                xt1 -= j as usize;
                xt2 = xt1 + (mh << 1) as usize;

                (vi, vr) = cplx_mult_div2(x[xt2], x[xt2 + 1], cs.re, cs.im);

                ur = half(x[xt1]);
                ui = half(x[xt1 + 1]);

                x[xt1] = ur.wrapping_add(vr);
                x[xt1 + 1] = ui.wrapping_sub(vi);

                x[xt2] = ur.wrapping_sub(vr);
                x[xt2 + 1] = ui.wrapping_add(vi);

                xt1 += mh as usize;
                xt2 += mh as usize;

                (vr, vi) = cplx_mult_div2(x[xt2], x[xt2 + 1], cs.re, cs.im);

                ur = half(x[xt1]);
                ui = half(x[xt1 + 1]);

                x[xt1] = ur.wrapping_sub(vr);
                x[xt1 + 1] = ui.wrapping_sub(vi);

                x[xt2] = ur.wrapping_add(vr);
                x[xt2 + 1] = ui.wrapping_add(vi);

                xt1 = xt2 + j as usize;

                r -= mh << 1;
                if r == 0 {
                    break;
                }
            }
            j += 4;
        }
        {
            let mut xt1 = (mh >> 1) as usize;
            let mut r: i32 = n;
            let w = stc(0x5a82799a);

            loop {
                let mut xt2 = xt1 + (mh << 1) as usize;

                let (mut vi, mut vr) = cplx_mult_div2(x[xt2 + 1], x[xt2], w, w);

                let mut ur = half(x[xt1]);
                let mut ui = half(x[xt1 + 1]);

                x[xt1] = ur.wrapping_add(vr);
                x[xt1 + 1] = ui.wrapping_add(vi);

                x[xt2] = ur.wrapping_sub(vr);
                x[xt2 + 1] = ui.wrapping_sub(vi);

                xt1 += mh as usize;
                xt2 += mh as usize;

                (vr, vi) = cplx_mult_div2(x[xt2 + 1], x[xt2], w, w);

                ur = half(x[xt1]);
                ui = half(x[xt1 + 1]);

                x[xt1] = ur.wrapping_add(vr);
                x[xt1 + 1] = ui.wrapping_sub(vi);

                x[xt2] = ur.wrapping_sub(vr);
                x[xt2 + 1] = ui.wrapping_add(vi);

                xt1 = xt2 + mh as usize;

                r -= mh << 1;
                if r == 0 {
                    break;
                }
            }
        }
        ldm -= 1;
        if ldm == 0 {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fft::testutil::*;
    use crate::tools_rom::{SINE_TABLE_1024, SINE_TABLE_512};

    /// dit_fft (NEON or unchecked scalar) against the straight port of the selected variant,
    /// with the tables of the library and others (other twiddle steps).
    #[test]
    fn dit_fft_equals_straight_port() {
        let mut r = Rng(0x5eed_0004);
        let configs: &[(i32, &[FixpStp], i32)] = &[
            (3, &SINE_TABLE_512, 512),
            (4, &SINE_TABLE_512, 512),
            (5, &SINE_TABLE_512, 512),
            (6, &SINE_TABLE_512, 512),
            (7, &SINE_TABLE_512, 512),
            (8, &SINE_TABLE_512, 512),
            (9, &SINE_TABLE_512, 512),
            (10, &SINE_TABLE_1024, 1024),
            (6, &SINE_TABLE_1024, 1024),
            (8, &SINE_TABLE_512, 256),
        ];
        for &(ldn, table, size) in configs {
            let n = 2usize << ldn;
            for k in 0..if n <= 128 { 3000 } else { 400 } {
                let x = signal(&mut r, k, n);
                let (mut a, mut b, mut c) = (x.clone(), x.clone(), x.clone());
                dit_fft(&mut a, ldn, table, size);
                dit_fft_scalar(&mut c, ldn, table, size);
                if ARCH_ARM {
                    dit_fft_arm(&mut b, ldn, table, size);
                } else {
                    dit_fft_generic(&mut b, ldn, table, size);
                }
                assert_eq!(a, b, "dit_fft(ldn {ldn}, size {size}) case {k}");
                assert_eq!(c, b, "dit_fft_scalar(ldn {ldn}, size {size}) case {k}");
            }
        }
    }
}
