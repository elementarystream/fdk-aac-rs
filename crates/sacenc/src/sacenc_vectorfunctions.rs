// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libSACenc/src/sacenc_vectorfunctions.h, libSACenc/src/sacenc_vectorfunctions.cpp.

//! Port of `sacenc_vectorfunctions.h` and `sacenc_vectorfunctions.cpp`: energies, scalar
//! products, correlation, coherence and headroom of complex vectors.
//!
//! Two-dimensional arrays (`const FIXP_DPK *const *x`, indexed `x[i][j]`) are slices of rows
//! (`&[R]` with `R: AsRef<[FixpDpk]>`, e.g. `&[Vec<FixpDpk>]`).
//!
//! # Variants
//!
//! The function bodies are identical in the `aarch64` and `x86` builds apart from the expansion
//! of `SATURATE_LEFT_SHIFT` (written differently, same values, see
//! [`fdk_aac_rs_fdk::scale::saturate_left_shift`]); the results differ through the primitives
//! (`fMult`, `invSqrtNorm2`, `sqrtFixp`).
//!
//! # Undefined behavior
//!
//! The right shifts by `2 * cs` (`sumUpCplxPow2`, `sumUpCplxPow2Dim2`) and by `sx + sy`
//! (`cplx_cplxScalarProduct`) reach 32 to 62 for large scale factors (not with the scale factors
//! of the library). Neither build vectorizes these loops: arm64 uses `asr w, w, w` and x86
//! `sarl %cl`, which both take the amount modulo 32, so the port uses `wrapping_shr`. Additions
//! that can overflow (sums of products) wrap in both builds (`add`/`addl`).

use fdk_aac_rs_fdk::common_fix::*;

/// `SUM_UP_STATIC_SCALE`
pub const SUM_UP_STATIC_SCALE: i32 = 0;
/// `SUM_UP_DYNAMIC_SCALE`
pub const SUM_UP_DYNAMIC_SCALE: i32 = 1;

/// `fixMax(fixMin(x, DFRACT_BITS - 1), -(DFRACT_BITS - 1))`
#[inline(always)]
fn clamp31(x: i32) -> i32 {
    fix_max(fix_min(x, DFRACT_BITS - 1), -(DFRACT_BITS - 1))
}

/// `sumUpCplxPow2(x, scaleMode, inScaleFactor, outScaleFactor, n)`: the sum of the energies of
/// `x[0..n]`, scaled by `outScaleFactor`.
pub fn sum_up_cplx_pow2(
    x: &[FixpDpk],
    scale_mode: i32,
    in_scale_factor: i32,
    out_scale_factor: &mut i32,
    n: i32,
) -> FixpDbl {
    let n = n.max(0) as usize;
    let mut cs;

    if scale_mode == SUM_UP_DYNAMIC_SCALE {
        // calculate headroom
        let mut max_val: FixpDbl = 0;
        for v in &x[..n] {
            max_val |= f_abs(v.re);
            max_val |= f_abs(v.im);
        }
        cs = in_scale_factor.wrapping_sub(fix_max(0, cnt_leading_zeros(max_val) - 1));
    } else {
        cs = in_scale_factor;
    }

    // consider scaling of energy and scaling in fPow2Div2 and addition
    *out_scale_factor = cs.wrapping_mul(2).wrapping_add(2);

    // make sure that the scalefactor is in the range of -(DFRACT_BITS-1), ..., (DFRACT_BITS-1)
    cs = clamp31(cs);

    // sum up complex energy samples
    let mut re: FixpDbl = 0;
    let mut im: FixpDbl = 0;
    if cs < 0 {
        cs = -cs;
        for v in &x[..n] {
            re = re.wrapping_add(f_pow2_div2(v.re.wrapping_shl(cs as u32)));
            im = im.wrapping_add(f_pow2_div2(v.im.wrapping_shl(cs as u32)));
        }
    } else {
        // `>> cs` with cs = 2 * cs up to 62: modulo 32 (see the module documentation).
        cs *= 2;
        for v in &x[..n] {
            re = re.wrapping_add(f_pow2_div2(v.re).wrapping_shr(cs as u32));
            im = im.wrapping_add(f_pow2_div2(v.im).wrapping_shr(cs as u32));
        }
    }

    (re >> 1) + (im >> 1)
}

/// `sumUpCplxPow2Dim2(x, scaleMode, inScaleFactor, outScaleFactor, sDim1, nDim1, sDim2, nDim2)`:
/// the sum of the energies of `x[sDim1..nDim1][sDim2..nDim2]`, scaled by `outScaleFactor`.
pub fn sum_up_cplx_pow2_dim2<R: AsRef<[FixpDpk]>>(
    x: &[R],
    scale_mode: i32,
    in_scale_factor: i32,
    out_scale_factor: &mut i32,
    s_dim1: i32,
    n_dim1: i32,
    s_dim2: i32,
    n_dim2: i32,
) -> FixpDbl {
    let mut cs;

    if scale_mode == SUM_UP_DYNAMIC_SCALE {
        // calculate headroom
        let mut max_val: FixpDbl = 0;
        for i in s_dim1..n_dim1 {
            let row = x[i as usize].as_ref();
            for j in s_dim2..n_dim2 {
                max_val |= f_abs(row[j as usize].re);
                max_val |= f_abs(row[j as usize].im);
            }
        }
        cs = in_scale_factor.wrapping_sub(fix_max(0, cnt_leading_zeros(max_val) - 1));
    } else {
        cs = in_scale_factor;
    }

    // consider scaling of energy and scaling in fPow2Div2 and addition
    *out_scale_factor = cs.wrapping_mul(2).wrapping_add(2);

    // make sure that the scalefactor is in the range of -(DFRACT_BITS-1), ..., (DFRACT_BITS-1)
    cs = clamp31(cs);

    // sum up complex energy samples
    let mut re: FixpDbl = 0;
    let mut im: FixpDbl = 0;
    if cs < 0 {
        cs = -cs;
        for i in s_dim1..n_dim1 {
            let row = x[i as usize].as_ref();
            for j in s_dim2..n_dim2 {
                let v = row[j as usize];
                re = re.wrapping_add(f_pow2_div2(v.re.wrapping_shl(cs as u32)));
                im = im.wrapping_add(f_pow2_div2(v.im.wrapping_shl(cs as u32)));
            }
        }
    } else {
        // `>> cs` with cs = 2 * cs up to 62: modulo 32 (see the module documentation).
        cs *= 2;
        for i in s_dim1..n_dim1 {
            let row = x[i as usize].as_ref();
            for j in s_dim2..n_dim2 {
                let v = row[j as usize];
                re = re.wrapping_add(f_pow2_div2(v.re).wrapping_shr(cs as u32));
                im = im.wrapping_add(f_pow2_div2(v.im).wrapping_shr(cs as u32));
            }
        }
    }

    (re >> 1) + (im >> 1)
}

/// `copyCplxVec(Z, X, n)`: `Z[0..n] = X[0..n]` (`FDKmemmove`).
pub fn copy_cplx_vec(z: &mut [FixpDpk], x: &[FixpDpk], n: i32) {
    let n = n.max(0) as usize;
    z[..n].copy_from_slice(&x[..n]);
}

/// `setCplxVec(Z, a, n)`: sets the real and imaginary parts of `Z[0..n]` to `a`.
pub fn set_cplx_vec(z: &mut [FixpDpk], a: FixpDbl, n: i32) {
    for v in z.iter_mut().take(n.max(0) as usize) {
        v.re = a;
        v.im = a;
    }
}

/// `cplx_cplxScalarProduct(Z, X, Y, scaleX, scaleY, scaleZ, sDim1, nDim1, sDim2, nDim2)`: the
/// complex scalar product of `X` and `Y` over `[sDim1..nDim1][sDim2..nDim2]`, scaled by
/// `scaleZ`.
pub fn cplx_cplx_scalar_product<R: AsRef<[FixpDpk]>, S: AsRef<[FixpDpk]>>(
    z: &mut FixpDpk,
    x: &[R],
    y: &[S],
    scale_x: i32,
    scale_y: i32,
    scale_z: &mut i32,
    s_dim1: i32,
    n_dim1: i32,
    s_dim2: i32,
    n_dim2: i32,
) {
    // make sure that the scalefactor is in the range of -(DFRACT_BITS-1), ..., (DFRACT_BITS-1)
    let mut sx = clamp31(scale_x);
    let mut sy = clamp31(scale_y);

    // consider scaling of energy and scaling in fMultDiv2 and shift of result values
    *scale_z = sx + sy + 2;

    let mut re: FixpDbl = 0;
    let mut im: FixpDbl = 0;

    // One product term: (fMultDiv2(a, b) +/- fMultDiv2(c, d)), computed in 32 bits.
    let term = |a: FixpDbl, b: FixpDbl, c: FixpDbl, d: FixpDbl, plus: bool| -> FixpDbl {
        if plus {
            f_mult_div2(a, b).wrapping_add(f_mult_div2(c, d))
        } else {
            f_mult_div2(a, b).wrapping_sub(f_mult_div2(c, d))
        }
    };

    if (sx < 0) && (sy < 0) {
        sx = -sx;
        sy = -sy;
        for i in s_dim1..n_dim1 {
            let (xr, yr) = (x[i as usize].as_ref(), y[i as usize].as_ref());
            for j in s_dim2..n_dim2 {
                let (xv, yv) = (xr[j as usize], yr[j as usize]);
                let xre = xv.re.wrapping_shl(sx as u32);
                let xim = xv.im.wrapping_shl(sx as u32);
                let yre = yv.re.wrapping_shl(sy as u32);
                let yim = yv.im.wrapping_shl(sy as u32);
                re = re.wrapping_add(term(xre, yre, xim, yim, true));
                im = im.wrapping_add(term(xim, yre, xre, yim, false));
            }
        }
    } else if (sx >= 0) && (sy >= 0) {
        // `>> (sx + sy)` up to 62: modulo 32 (see the module documentation).
        let s = (sx + sy) as u32;
        for i in s_dim1..n_dim1 {
            let (xr, yr) = (x[i as usize].as_ref(), y[i as usize].as_ref());
            for j in s_dim2..n_dim2 {
                let (xv, yv) = (xr[j as usize], yr[j as usize]);
                let (xre, xim, yre, yim) = (xv.re, xv.im, yv.re, yv.im);
                re = re.wrapping_add(term(xre, yre, xim, yim, true).wrapping_shr(s));
                im = im.wrapping_add(term(xim, yre, xre, yim, false).wrapping_shr(s));
            }
        }
    } else if (sx < 0) && (sy >= 0) {
        sx = -sx;
        for i in s_dim1..n_dim1 {
            let (xr, yr) = (x[i as usize].as_ref(), y[i as usize].as_ref());
            for j in s_dim2..n_dim2 {
                let (xv, yv) = (xr[j as usize], yr[j as usize]);
                let xre = xv.re.wrapping_shl(sx as u32);
                let xim = xv.im.wrapping_shl(sx as u32);
                let (yre, yim) = (yv.re, yv.im);
                re = re.wrapping_add(term(xre, yre, xim, yim, true) >> sy);
                im = im.wrapping_add(term(xim, yre, xre, yim, false) >> sy);
            }
        }
    } else {
        sy = -sy;
        for i in s_dim1..n_dim1 {
            let (xr, yr) = (x[i as usize].as_ref(), y[i as usize].as_ref());
            for j in s_dim2..n_dim2 {
                let (xv, yv) = (xr[j as usize], yr[j as usize]);
                let (xre, xim) = (xv.re, xv.im);
                let yre = yv.re.wrapping_shl(sy as u32);
                let yim = yv.im.wrapping_shl(sy as u32);
                re = re.wrapping_add(term(xre, yre, xim, yim, true) >> sx);
                im = im.wrapping_add(term(xim, yre, xre, yim, false) >> sx);
            }
        }
    }

    z.re = re >> 1;
    z.im = im >> 1;
}

/// `FDKcalcCorrelationVec(z, pr12, p1, p2, n)`: `z[i] = pr12[i] / sqrt(p1[i] * p2[i])`.
pub fn fdk_calc_correlation_vec(
    z: &mut [FixpDbl],
    pr12: &[FixpDbl],
    p1: &[FixpDbl],
    p2: &[FixpDbl],
    n: i32,
) {
    for i in 0..n.max(0) as usize {
        let mut p12 = f_mult(p1[i], p2[i]);
        if p12 > 0 {
            let mut s = 0;
            p12 = inv_sqrt_norm2(p12, &mut s);
            let cor = f_mult(pr12[i], p12);
            z[i] = saturate_left_shift(cor, s, DFRACT_BITS);
        } else {
            z[i] = MAXVAL_DBL;
        }
    }
}

/// `calcCoherenceVec(z, p12r, p12i, p1, p2, scaleP12, scaleP, n)`:
/// `z[i] = sqrt((p12r[i]^2 + p12i[i]^2) / (p1[i] * p2[i]))`.
pub fn calc_coherence_vec(
    z: &mut [FixpDbl],
    p12r: &[FixpDbl],
    p12i: &[FixpDbl],
    p1: &[FixpDbl],
    p2: &[FixpDbl],
    scale_p12: i32,
    scale_p: i32,
    n: i32,
) {
    for i in 0..n.max(0) as usize {
        let s2 = fix_min(
            fix_max(0, count_leading_bits(p12r[i]) - 1),
            fix_max(0, count_leading_bits(p12i[i]) - 1),
        );
        let p12ri = sqrt_fixp(f_pow2_div2(p12r[i] << s2).wrapping_add(f_pow2_div2(p12i[i] << s2)));
        let s1 = fix_min(
            fix_max(0, count_leading_bits(p1[i]) - 1),
            fix_max(0, count_leading_bits(p2[i]) - 1),
        );
        let mut p12 = f_mult_div2(p1[i] << s1, p2[i] << s1);

        if p12 > 0 {
            let mut s = 0;
            p12 = inv_sqrt_norm2(p12, &mut s);
            let coh = f_mult(p12ri, p12);
            s = clamp31(
                scale_p12.wrapping_sub(scale_p).wrapping_add(s).wrapping_add(s1).wrapping_sub(s2),
            );
            if s < 0 {
                z[i] = coh >> (-s);
            } else {
                z[i] = saturate_left_shift(coh, s, DFRACT_BITS);
            }
        } else {
            z[i] = MAXVAL_DBL;
        }
    }
}

/// `addWeightedCplxVec(Z, a, X, b, Y, scale, scaleCh1, scaleCh2,
/// pParameterBand2HybridBandOffset, nParameterBands, nTimeSlots, startTimeSlot)`:
/// `Z[j][i] = a[pb] * X[j][i] + b[pb] * Y[j][i]`. Updates `scaleCh1` with the common scale of
/// both channels.
pub fn add_weighted_cplx_vec<W: AsMut<[FixpDpk]>, R: AsRef<[FixpDpk]>, S: AsRef<[FixpDpk]>>(
    z: &mut [W],
    a: &[FixpDbl],
    x: &[R],
    b: &[FixpDbl],
    y: &[S],
    scale: i32,
    scale_ch1: &mut i32,
    scale_ch2: i32,
    p_parameter_band2_hybrid_band_offset: &[u8],
    n_parameter_bands: i32,
    n_time_slots: i32,
    start_time_slot: i32,
) {
    // determine maximum scale of both channels
    let cs = fix_max(*scale_ch1, scale_ch2);
    let s1 = cs.wrapping_sub(*scale_ch1) as u32;
    let s2 = cs.wrapping_sub(scale_ch2) as u32;

    // scalefactor 1 is updated with common scale of channel 1 and channel2
    *scale_ch1 = cs;

    let sh = scale.wrapping_add(1) as u32;
    let mut j = 0usize;
    for pb in 0..n_parameter_bands.max(0) as usize {
        let a_pb = a[pb];
        let b_pb = b[pb];
        while j < p_parameter_band2_hybrid_band_offset[pb] as usize {
            let (zr, xr, yr) = (z[j].as_mut(), x[j].as_ref(), y[j].as_ref());
            for i in start_time_slot..n_time_slots {
                let i = i as usize;
                zr[i].re = (f_mult_div2(a_pb, xr[i].re)
                    .wrapping_shr(s1)
                    .wrapping_add(f_mult_div2(b_pb, yr[i].re).wrapping_shr(s2)))
                .wrapping_shl(sh);
                zr[i].im = (f_mult_div2(a_pb, xr[i].im)
                    .wrapping_shr(s1)
                    .wrapping_add(f_mult_div2(b_pb, yr[i].im).wrapping_shr(s2)))
                .wrapping_shl(sh);
            }
            j += 1;
        }
    }
}

/// `FDKcalcPbScaleFactor(x, pParameterBand2HybridBandOffset, outScaleFactor, startTimeSlot,
/// nTimeSlots, nParamBands)`: the headroom of `x[startTimeSlot..nTimeSlots][...]` in every
/// parameter band.
pub fn fdk_calc_pb_scale_factor<R: AsRef<[FixpDpk]>>(
    x: &[R],
    p_parameter_band2_hybrid_band_offset: &[u8],
    out_scale_factor: &mut [i32],
    start_time_slot: i32,
    n_time_slots: i32,
    n_param_bands: i32,
) {
    // calculate headroom
    let mut j = 0usize;
    for pb in 0..n_param_bands.max(0) as usize {
        let mut max_val: FixpDbl = 0;
        while j < p_parameter_band2_hybrid_band_offset[pb] as usize {
            for i in start_time_slot..n_time_slots {
                let v = x[i as usize].as_ref()[j];
                max_val |= f_abs(v.re);
                max_val |= f_abs(v.im);
            }
            j += 1;
        }
        out_scale_factor[pb] = -fix_max(0, cnt_leading_zeros(max_val) - 1);
    }
}

/// `FDKcalcScaleFactor(x, y, n)`: the common headroom of `x[0..n]` and `y[0..n]` (`None` is
/// `NULL`).
pub fn fdk_calc_scale_factor(x: Option<&[FixpDbl]>, y: Option<&[FixpDbl]>, n: i32) -> i32 {
    let n = n.max(0) as usize;
    // calculate headroom
    let mut max_val: FixpDbl = 0;
    if let Some(x) = x {
        for &v in &x[..n] {
            max_val |= f_abs(v);
        }
    }

    if let Some(y) = y {
        for &v in &y[..n] {
            max_val |= f_abs(v);
        }
    }

    if max_val == 0 {
        -(DFRACT_BITS - 1)
    } else {
        // `-CountLeadingBits(maxVal)`: both builds assume that `maxVal`, an OR of `fAbs()`
        // values, is not negative (`-x` does not overflow) and compute `1 - clz(maxVal)`
        // (arm64 `clz`, x86 `bsr`). With `fAbs(MINVAL_DBL)` among the values, `maxVal` is
        // negative and the result is 1, where `-fixnorm_D(maxVal)` would give a value <= 0.
        1 - cnt_leading_zeros(max_val)
    }
}

/// `FDKcalcScaleFactorDPK(x, startBand, bands)`: the headroom of `x[startBand..bands]`.
pub fn fdk_calc_scale_factor_dpk(x: &[FixpDpk], start_band: i32, bands: i32) -> i32 {
    let mut max_val: FixpDbl = 0;

    for qs in start_band..bands {
        max_val |= f_abs(x[qs as usize].re);
        max_val |= f_abs(x[qs as usize].im);
    }

    -fix_max(0, cnt_leading_zeros(max_val) - 1)
}

/// `FDKmemcpy_flex(dst, dstStride, src, srcStride, nSamples)`:
/// `dst[i * dstStride] = src[i * srcStride]`. `dst` and `src` are the slices that the C
/// pointers point into, `dst_off`/`src_off` the positions of the pointers.
#[inline]
pub fn fdk_memcpy_flex<T: Copy>(
    dst: &mut [T],
    dst_off: usize,
    dst_stride: i32,
    src: &[T],
    src_off: usize,
    src_stride: i32,
    n_samples: i32,
) {
    for i in 0..n_samples.max(0) as isize {
        let d = dst_off as isize + i * dst_stride as isize;
        let s = src_off as isize + i * src_stride as isize;
        dst[d as usize] = src[s as usize];
    }
}

/// `FDKmemset_flex(x, c, nSamples)`: `x[0..nSamples] = c`.
#[inline]
pub fn fdk_memset_flex<T: Copy>(x: &mut [T], c: T, n_samples: i32) {
    for v in x.iter_mut().take(n_samples.max(0) as usize) {
        *v = c;
    }
}
