// fdk-aac-rs: Third-Party Modified Version of the Fraunhofer FDK AAC Codec Library for Android.
// A Rust port of fdk-aac 2.0.3. See LICENSE.txt for the complete license text.
//
// Ported from libSBRdec/src/HFgen_preFlat.h, libSBRdec/src/HFgen_preFlat.cpp.

//! Port of `HFgen_preFlat.h` and `HFgen_preFlat.cpp`: QMF frequency pre-whitening for SBR. A
//! gain vector flattens the spectral envelope of the low band (a polynomial fit of the band
//! energies in dB) without losing the fine structure.
//!
//! # Undefined behavior
//!
//! The exponent alignments of `polyfit()`, `backsubst_fw()` and `backsubst_bw()` shift by
//! amounts that are not limited (e.g. `sum >>= diff`, `x[0] >> (sum_sf - x_sf[0] + 2)`); both
//! builds compile them to register shifts (`asr`/`lsl` on AArch64, `sar`/`shl` with `cl` on
//! x86), which take the amount modulo 32, so the port uses `wrapping_shr`/`wrapping_shl`. The
//! sums can overflow (signed overflow, 32 bit wrapping instructions): `wrapping_add`/`sub`.

pub mod tables;
use fdk_aac_rs_fdk::fdk_assert;
use tables::BSD;
pub use tables::GET_LOG2;

use fdk_aac_rs_fdk::common_fix::*;

/// `GAIN_VEC_EXP`: exponent of `GainVec[]`
pub const GAIN_VEC_EXP: i32 = 6;

/// `POLY_ORDER`
const POLY_ORDER: usize = 3;
/// `MAXLOWBANDS`
const MAXLOWBANDS: usize = 32;
/// `BSD_IDX_OFFSET`: `bsd[]` begins at index 0 with data for numBands=5.
const BSD_IDX_OFFSET: i32 = 5;
/// `SUM_SAFETY`
const SUM_SAFETY: i32 = 2;

/// `LOG10FAC` (`== 10/log2(10) * 2^-2`)
const LOG10FAC: f32 = 0.752574989159953f32;
/// `LOG10FAC_INV` (`== log2(10)/20 * 2^2`)
const LOG10FAC_INV: f32 = 0.664385618977472f32;

/// `backsubst_data`
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BacksubstData {
    /// `Lnorm1d`: normalized L matrix
    pub lnorm1d: [FixpSgl; 3],
    /// `Lnorm1d_sf`
    pub lnorm1d_sf: [i8; 3],
    /// `Lnormii`: the diagonal data points `[i][i]` of the normalized L matrix
    pub lnormii: [FixpSgl; 3],
    /// `Lnormii_sf`
    pub lnormii_sf: [i8; 3],
    /// `Bmul0`: to normalize L*x=b, Bmul0 is what we need to multiply b with.
    pub bmul0: [FixpSgl; 4],
    /// `Bmul0_sf`
    pub bmul0_sf: [i8; 4],
    /// `LnormInv1d`: normalized inverted L matrix (L')
    pub lnorm_inv1d: [FixpSgl; 6],
    /// `LnormInv1d_sf`
    pub lnorm_inv1d_sf: [i8; 6],
    /// `Bmul1`: to normalize L'*x=b, Bmul1 is what we need to multiply b with.
    pub bmul1: [FixpSgl; 4],
    /// `Bmul1_sf`
    pub bmul1_sf: [i8; 4],
}

/// The `bsd[]` entry of `numBands` (`BSD_IDX_OFFSET <= numBands <= MAXLOWBANDS`).
///
/// For other values the reference has no defined result; the port clamps the index.
fn bsd(num_bands: i32) -> &'static BacksubstData {
    let idx = (num_bands - BSD_IDX_OFFSET).clamp(0, BSD.len() as i32 - 1);
    &BSD[idx as usize]
}

/// `backsubst_fw()`: solves L*x=b via backsubstitution.
pub fn backsubst_fw(num_bands: i32, b: &[FixpDbl], x: &mut [FixpDbl], x_sf: &mut [i32]) {
    let d = bsd(num_bands);
    let p_lnorm1d = &d.lnorm1d;
    let p_lnorm1d_sf = &d.lnorm1d_sf;
    let p_lnormii = &d.lnormii;
    let p_lnormii_sf = &d.lnormii_sf;

    x[0] = b[0];

    let mut m = 0usize; // the trip counter that indexes incrementally through Lnorm1d[]
    for i in 1..=POLY_ORDER {
        let mut sum = b[i] >> SUM_SAFETY;
        let mut sum_sf = x_sf[i];
        let mut k = i - 1;
        while k > 0 {
            let mut e = 0;
            let mut mult = f_mult_norm(fx_sgl2fx_dbl(p_lnorm1d[m]), x[k], &mut e);
            let mult_sf = p_lnorm1d_sf[m] as i32 + x_sf[k] + e;

            // check if the new summand mult has a different sf than the sum currently has
            let diff = mult_sf - sum_sf;

            if diff > 0 {
                // yes, and it requires the sum to be adjusted (scaled down)
                sum = sum.wrapping_shr(diff as u32);
                sum_sf = mult_sf;
            } else if diff < 0 {
                // yes, but here mult needs to be scaled down
                mult = mult.wrapping_shr((-diff) as u32);
            }
            sum = sum.wrapping_sub(mult >> SUM_SAFETY);
            k -= 1;
            m += 1;
        }

        // - x[0]
        if x_sf[0] > sum_sf {
            sum = sum.wrapping_shr((x_sf[0] - sum_sf) as u32);
            sum_sf = x_sf[0];
        }
        sum = sum.wrapping_sub(x[0].wrapping_shr((sum_sf - x_sf[0] + SUM_SAFETY) as u32));

        // instead of the division /L[i][i], we multiply by the inverse
        let mut e = 0;
        x[i] = f_mult_norm(sum, fx_sgl2fx_dbl(p_lnormii[i - 1]), &mut e);
        x_sf[i] = sum_sf + p_lnormii_sf[i - 1] as i32 + e + SUM_SAFETY;
    }
}

/// `backsubst_bw()`: solves L*x=b via backsubstitution (backwards).
pub fn backsubst_bw(num_bands: i32, b: &[FixpDbl], x: &mut [FixpDbl], x_sf: &mut [i32]) {
    let d = bsd(num_bands);
    let p_lnorm_inv1d = &d.lnorm_inv1d;
    let p_lnorm_inv1d_sf = &d.lnorm_inv1d_sf;

    x[POLY_ORDER] = b[POLY_ORDER];

    let mut m = 0usize; // the trip counter that indexes incrementally through LnormInv1d[]
    for i in (0..POLY_ORDER).rev() {
        let mut sum = b[i] >> SUM_SAFETY;
        // sum's sf but disregarding SUM_SAFETY (added at the iteration's end)
        let mut sum_sf = x_sf[i];

        for k in i + 1..=POLY_ORDER {
            let mut e = 0;
            let mut mult = f_mult_norm(fx_sgl2fx_dbl(p_lnorm_inv1d[m]), x[k], &mut e);
            let mult_sf = p_lnorm_inv1d_sf[m] as i32 + x_sf[k] + e;

            // check if the new summand mult has a different sf than sum currently has
            let diff = mult_sf - sum_sf;

            if diff > 0 {
                // yes, and it requires the sum v to be adjusted (scaled down)
                sum = sum.wrapping_shr(diff as u32);
                sum_sf = mult_sf;
            } else if diff < 0 {
                // yes, but here mult needs to be scaled down
                mult = mult.wrapping_shr((-diff) as u32);
            }

            // mult has now the same sf than what it is about to be added to. scale mult down
            // additionally so that building the sum is overflow-safe.
            sum = sum.wrapping_sub(mult >> SUM_SAFETY);
            m += 1;
        }

        x_sf[i] = sum_sf + SUM_SAFETY;
        x[i] = sum;
    }
}

/// `choleskySolve()`: solves a system of linear equations (L*x=b) with the Cholesky algorithm.
/// `b` is the input vector b and the output solution p; `b_sf` their exponents.
pub fn cholesky_solve(
    num_bands: i32,
    b: &mut [FixpDbl; POLY_ORDER + 1],
    b_sf: &mut [i32; POLY_ORDER + 1],
) {
    let d = bsd(num_bands);
    let p_bmul0 = &d.bmul0;
    let p_bmul0_sf = &d.bmul0_sf;
    let p_bmul1 = &d.bmul1;
    let p_bmul1_sf = &d.bmul1_sf;

    // normalize b
    let mut bnormed = [0 as FixpDbl; POLY_ORDER + 1];
    for i in 0..=POLY_ORDER {
        let mut e = 0;
        bnormed[i] = f_mult_norm(b[i], fx_sgl2fx_dbl(p_bmul0[i]), &mut e);
        b_sf[i] += p_bmul0_sf[i] as i32 + e;
    }

    backsubst_fw(num_bands, &bnormed, b, b_sf);

    // normalize b again
    for i in 0..=POLY_ORDER {
        let mut e = 0;
        bnormed[i] = f_mult_norm(b[i], fx_sgl2fx_dbl(p_bmul1[i]), &mut e);
        b_sf[i] += p_bmul1_sf[i] as i32 + e;
    }

    backsubst_bw(num_bands, &bnormed, b, b_sf);
}

/// `polyfit()`: polynomial approximation of `y` with the implicit abscissas `0..n-1`
/// (`BSD_IDX_OFFSET <= numBands <= MAXLOWBANDS`).
pub fn polyfit(
    num_bands: i32,
    y: &[FixpDbl],
    y_sf: i32,
    p: &mut [FixpDbl; POLY_ORDER + 1],
    p_sf: &mut [i32; POLY_ORDER + 1],
) {
    let mut v = [0i32; POLY_ORDER + 1];
    let sum_saftey = GET_LOG2[(num_bands - 1).clamp(0, 31) as usize] as i32;

    fdk_assert!((BSD_IDX_OFFSET..=MAXLOWBANDS as i32).contains(&num_bands));

    // construct vector b[] temporarily stored in array p[]
    p.fill(0);

    // p[] are the sums over n values and each p[i] has its own sf
    p_sf.fill(1 - DFRACT_BITS);

    for k in 0..num_bands {
        v[0] = 1;
        for i in 1..=POLY_ORDER {
            v[i] = k.wrapping_mul(v[i - 1]);
        }

        for i in 0..=POLY_ORDER {
            if v[POLY_ORDER - i] != 0 && y[k as usize] != 0 {
                let mut e = 0;
                let mut mult = f_mult_norm(v[POLY_ORDER - i], y[k as usize], &mut e);
                let sf = DFRACT_BITS - 1 + y_sf + e;

                // check if the new summand has a different sf than the sum p[i] currently has
                let diff = sf - p_sf[i];

                if diff > 0 {
                    // yes, and it requires the sum p[i] to be adjusted (scaled down)
                    p[i] >>= f_min(DFRACT_BITS - 1, diff);
                    p_sf[i] = sf;
                } else if diff < 0 {
                    // yes, but here mult needs to be scaled down
                    mult = mult.wrapping_shr((-diff) as u32);
                }

                // mult has now the same sf than what it is about to be added to. scale mult down
                // additionally so that building the sum is overflow-safe.
                p[i] = p[i].wrapping_add(mult >> sum_saftey);
            }
        }
    }

    p_sf[0] += sum_saftey;
    p_sf[1] += sum_saftey;
    p_sf[2] += sum_saftey;
    p_sf[3] += sum_saftey;

    cholesky_solve(num_bands, p, p_sf);
}

/// `polyval()`: the value of the POLY_ORDER-degree polynomial at the integer `x_int` (Horner
/// scheme), and its exponent.
pub fn polyval(
    p: &[FixpDbl; POLY_ORDER + 1],
    p_sf: &[i32; POLY_ORDER + 1],
    x_int: i32,
    out_sf: &mut i32,
) -> FixpDbl {
    fdk_assert!(x_int <= 31); // otherwise getLog2[] needs more elements

    // if x == 0, then y(x) is just p3
    if x_int == 0 {
        *out_sf = p_sf[3];
        return p[3];
    }
    let x_sf = GET_LOG2[(x_int as usize).min(31)] as i32;
    let x: FixpDbl = x_int << (DFRACT_BITS - 1 - x_sf);

    let mut result = p[0];
    let mut result_sf = p_sf[0];

    for k in 1..=POLY_ORDER {
        let mut mult = f_mult(x, result);
        let mut mult_sf = x_sf + result_sf;

        let room = count_leading_bits(mult);
        mult <<= room;
        mult_sf -= room;

        let mut pp = p[k];
        let mut pp_sf = p_sf[k];

        // equalize the shift factors of pp and mult so that we can sum them up
        let mut diff = pp_sf - mult_sf;

        if diff > 0 {
            diff = f_min(diff, DFRACT_BITS - 1);
            mult >>= diff;
        } else if diff < 0 {
            diff = f_max(diff, 1 - DFRACT_BITS);
            pp >>= -diff;
        }

        // downshift by 1 to ensure safe summation
        mult >>= 1;
        mult_sf += 1;
        pp >>= 1;
        pp_sf += 1;

        result_sf = f_max(pp_sf, mult_sf);

        result = mult.wrapping_add(pp);
    }

    *out_sf = result_sf;
    result
}

/// `sourceBufferImag[i][band]`; the imaginary buffer is `NULL` (empty) in low power mode, where
/// the reference has no defined result. Deviation: the port reads 0.
#[inline(always)]
fn imag_at(imag: &[&mut [FixpDbl]], i: i32, band: usize) -> FixpDbl {
    imag.get(i as usize).and_then(|s| s.get(band)).copied().unwrap_or(0)
}

/// `sbrDecoder_calculateGainVec()`: the gain vector that flattens the QMF bands `0..numBands`
/// without losing the fine structure. `gain_vec[i] * 2^gain_vec_exp[i]` is the gain of band `i`.
///
/// The slots `start_sample..stop_sample` of `source_buffer_real`/`source_buffer_imag` are
/// analyzed; the first `overlap` slots have the exponent `source_buf_e_overlap`, the others
/// `source_buf_e_current`.
#[allow(clippy::too_many_arguments)]
pub fn sbr_decoder_calculate_gain_vec(
    source_buffer_real: &[&mut [FixpDbl]],
    source_buffer_imag: &[&mut [FixpDbl]],
    source_buf_e_overlap: i32,
    source_buf_e_current: i32,
    overlap: i32,
    gain_vec: &mut [FixpDbl],
    gain_vec_exp: &mut [i32],
    num_bands: i32,
    start_sample: i32,
    stop_sample: i32,
) {
    let mut p = [0 as FixpDbl; POLY_ORDER + 1];
    let mut low_env = [0 as FixpDbl; MAXLOWBANDS];
    let inv_num_bands = get_inv_int(num_bands);
    let inv_num_slots = get_inv_int(stop_sample - start_sample);
    let mut sum_scale = 5;
    let mut sum_scale_ov = 3;

    if overlap > 8 {
        fdk_assert!(overlap <= 16);
        sum_scale_ov += 1;
        sum_scale += 1;
    }

    // exponents of energy values
    let source_buf_e_overlap = source_buf_e_overlap * 2 + sum_scale_ov;
    let source_buf_e_current = source_buf_e_current * 2 + sum_scale;
    let mut exp = f_max(source_buf_e_overlap, source_buf_e_current);
    let mut scale_nrg = source_buf_e_current - exp;
    let mut scale_nrg_ov = source_buf_e_overlap - exp;

    // numBands is a crossover band (at most 32); the port limits it to LowEnv[].
    let num_bands_c = (num_bands.max(0) as usize).min(MAXLOWBANDS);

    let mut mean_nrg: FixpDbl = 0;
    // Calculate the spectral envelope in dB over the current copy-up frame.
    for lo_band in 0..num_bands_c {
        let mut reserve = 0;
        let mut max_val: FixpDbl = 0;

        for i in start_sample.max(0)..stop_sample {
            let r = source_buffer_real[i as usize][lo_band];
            let im = imag_at(source_buffer_imag, i, lo_band);
            max_val |= r ^ (r >> (DFRACT_BITS - 1));
            max_val |= im ^ (im >> (DFRACT_BITS - 1));
        }

        if max_val != 0 {
            reserve = cnt_leading_zeros(max_val) - 2;
        }

        let mut nrg_ov: FixpDbl = 0;
        let mut nrg: FixpDbl = 0;
        if scale_nrg_ov > -31 {
            for i in start_sample.max(0)..overlap {
                let r = source_buffer_real[i as usize][lo_band];
                let im = imag_at(source_buffer_imag, i, lo_band);
                nrg_ov = nrg_ov.wrapping_add(
                    f_pow2_div2(scale_value(r, reserve))
                        .wrapping_add(f_pow2_div2(scale_value(im, reserve)))
                        >> sum_scale_ov,
                );
            }
        } else {
            scale_nrg_ov = 0;
        }
        if scale_nrg > -31 {
            for i in overlap.max(0)..stop_sample {
                let r = source_buffer_real[i as usize][lo_band];
                let im = imag_at(source_buffer_imag, i, lo_band);
                nrg = nrg.wrapping_add(
                    f_pow2_div2(scale_value(r, reserve))
                        .wrapping_add(f_pow2_div2(scale_value(im, reserve)))
                        >> sum_scale,
                );
            }
        } else {
            scale_nrg = 0;
        }

        nrg =
            (scale_value(nrg_ov, scale_nrg_ov) >> 1).wrapping_add(scale_value(nrg, scale_nrg) >> 1);
        nrg = f_mult(nrg, inv_num_slots);

        // +1 for addition directly above, +1 for fPow2Div2 in loops above
        let exp_new = exp - (2 * reserve) + 2;

        // LowEnv = 10*log10(nrg) = log2(nrg) * 10/log2(10)
        // exponent of logarithmic energy is 8
        if nrg > 0 {
            let mut exp_log2 = 0;
            nrg = calc_log2(nrg, exp_new, &mut exp_log2);
            nrg = scale_value(nrg, exp_log2 - 6);
            nrg = f_mult(fl2fxconst_sgl(LOG10FAC as f64), nrg);
        } else {
            nrg = 0;
        }
        low_env[lo_band] = nrg;
        mean_nrg = mean_nrg.wrapping_add(f_mult(nrg, inv_num_bands));
    }
    exp = 6 + 2; // exponent of LowEnv: +2 is exponent of LOG10FAC

    // subtract mean before polynomial approximation to reduce dynamic of p[]
    for v in low_env[..num_bands_c].iter_mut() {
        *v = mean_nrg.wrapping_sub(*v);
    }

    // For numBands < BSD_IDX_OFFSET (== POLY_ORDER+2) we dont get an overdetermined equation
    // system. The calculated polynomial will exactly fit the input data and evaluating the
    // polynomial will lead to the same vector than the original input vector:
    // lowEnvSlope[] == lowEnv[]
    if num_bands > POLY_ORDER as i32 + 1 {
        // Find polynomial approximation of LowEnv
        let mut p_sf = [0i32; POLY_ORDER + 1];

        polyfit(num_bands, &low_env, exp, &mut p, &mut p_sf);

        for i in 0..num_bands_c {
            let mut sf = 0;

            // lowBandEnvSlope[i] = tmp;
            let mut tmp = polyval(&p, &p_sf, i as i32, &mut sf);

            // GainVec = 10^((mean(y)-y)/20) = 2^( (mean(y)-y) * log2(10)/20 )
            tmp = f_mult(tmp, fl2fxconst_sgl(LOG10FAC_INV as f64));
            // -2 is exponent of LOG10FAC_INV
            gain_vec[i] = f2_pow(tmp, sf - 2, &mut gain_vec_exp[i]);
        }
    } else {
        // numBands <= POLY_ORDER+1
        for i in 0..num_bands_c {
            let sf = exp; // exponent of LowEnv[]

            // lowBandEnvSlope[i] = LowEnv[i];
            let mut tmp = low_env[i];

            // GainVec = 10^((mean(y)-y)/20) = 2^( (mean(y)-y) * log2(10)/20 )
            tmp = f_mult(tmp, fl2fxconst_sgl(LOG10FAC_INV as f64));
            // -2 is exponent of LOG10FAC_INV
            gain_vec[i] = f2_pow(tmp, sf - 2, &mut gain_vec_exp[i]);
        }
    }
}
